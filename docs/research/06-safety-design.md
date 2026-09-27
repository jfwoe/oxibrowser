# 06. 자격증명을 보유한 에이전트의 안전장치 설계

- 대상: OxiBrowser(pure-Rust 헤드리스, boa JS 엔진, CDP 서버, HAR 캡처, session/executor 구조) 기반 무인 자동화
- 배경 사례: Cloudflare 터널 인증, Tailscale 관리콘솔 승인, macOS iCloud 로그아웃, 시스템설정 GUI 승인에서 사용자 개입이 필수였던 흐름
- 목표: 사용자 자격증명(키체인 등)을 "에이전트가 대신 사용"하되, 유출·오용·피싱·무단 불가역 행동을 구조적으로 막는 설계

---

## 1. 위협 모델 먼저 정리

에이전트가 자격증명을 보유하는 순간 새로 생기는 위협은 기존 브라우저와 다르다:

| 위협 | 경로 | 이 보고서 대응 절 |
|---|---|---|
| 프롬프트 인젝션 | 페이지 본문/광고/댓글이 "설정에서 2FA를 꺼라" 등 지시 | §2, §5 |
| 피싱/도메인 사칭 | `cloudflare-verify.login.example`류look-alike에 자격증명 자동 입력 | §3 |
| 시크릿 유출 | HAR/트랜스크립트/스크린샷/로그에 비밀번호·토큰·쿠키 평문 기록 | §4 |
| 무단 불가역 행동 | 결제, 삭제, 이메일 발송, iCloud 로그아웃, 리소스 파기 | §5 |
| 세션 잔존 | 브라우저 종료 후에도 유효한 쿠키/리프레시 토큰 | §6 |
| 과권한 토큰 | 세션에서 캡처한 토큰은 원본 스코프 그대로 → 폭발 반경 확대 | §7 |

핵심 원칙(Chrome의 브라우저 에이전트 보안 가이드와 일치): **반환된 웹 콘텐츠는 신뢰하지 않는 입력**이고, **민감 행동은 출처(origin) 검증 + 명시적 확인**으로만 허용한다. 출처: <https://developer.chrome.com/docs/agents/security>

---

## 2. Per-site 자격증명 사용 동의와 감사 로그

### 2.1 동의는 "credential 단위 × origin 단위"로

Claude Code의 권한 모델이 참고할 만한 기성 패턴이다. 규칙 평가 순서가 **deny → 모드 → allow → 프롬프트**로, deny가 allow와 bypass보다 항상 우선한다. 즉 "허용 목록은 마찰을 줄이는 도구, 거부 목록이 불변식을 지키는 도구"다. 출처: <https://docs.anthropic.com/en/docs/claude-code/iam>

에이전트 자격증명에 옮기면:

```text
자격증명 사용 요청(origin, credential_id, purpose)
  1. DENY 목록에 일치 → 즉시 거부(로그만 남김)
  2. 이미 "always allow" 동의가 (credential_id, exact_origin) 쌍으로 존재 → 자동 진행
  3. 그 외 → 사용자 승인 프롬프트 (한 번 / 이 사이트 항상 / 거부)
```

동의 레코드 스키마 권고:

```json
{
  "credential_id": "keychain:cloudflare-tunnel",
  "origin": "https://dash.cloudflare.com",
  "actions": ["login"],              // login만, 또는 login+mfa
  "granted_at": "2026-09-27T10:00:00Z",
  "granted_by": "user:local",
  "expires_at": "2026-10-11T10:00:00Z",  // 무기한 동의 금지
  "max_uses": 50
}
```

- 동의의 최소 단위는 **credential × exact origin × action**(예: `login`), 사이트 전체(eTLD+1)가 아니다(§3 참조).
- 동의는 만료·횟수 제한이 있어야 한다. "항상 허용"이 영구적이면 피싱·계정 탈취 시 폭발 반경이 무한이 된다.
- 로컬 프롬프트 인젝션에 대비해, 동의 프롬프트에는 **페이지가 아니라 브로커가** origin을 표시한다. 페이지 콘텐츠(타이틀·파비콘·브랜드명)는 절대 표시 근거로 쓰지 않는다.

### 2.2 감사 로그: append-only, 무엇을 누가 언제

감사 로그는 에이전트가 지울 수 없는 별도 채널(파일/JSONL)에 쓴다. 최소 기록 항목:

```text
- credential_use:   credential_id, origin, action, decision(allow/deny/prompt), 이유
- credential_read:  키체인 조회 자체(성공/실패) — 조회만으로도 이상 징후
- sensitive_action: action_type, origin, 요약(금액/수신자), 확인 방식, 확인자
- session_teardown: 폐기된 쿠키 수, 폐기된 토큰 목록(해시만), 세션 지속 시간
- policy_violation: deny 매칭, origin 불일치 시도, 리다이렉트로 origin 변경 감지
```

- 로그에는 시크릿 값 대신 **자격증명 핸들과 값의 해시(SHA-256 앞 8자)**만 남긴다(§4).
- 승인 피로(approval fatigue)는 실증적으로 알려진 약점이다. CSA의 "GhostApproval" 연구는 6개 코딩 에이전트에서 "human-in-the-loop"가 명목상 보장과 실제 강제 사이의 간극을 보였고, 승인 프롬프트는 사람이 안 읽게 되어 보안 통제로서 실패한다는 오래된 증거를 재확인한다. 출처: <https://labs.cloudsecurityalliance.org> — 따라서 승인을 "유일한 방어선"으로 설계하지 않고 deny 규칙·origin 고정 같은 결정적 통제(deterministic enforcement)와 병행해야 한다.

---

## 3. 피싱 방지 — origin 고정과 비밀번호 매니저의 교훈

### 3.1 매칭은 계층적으로, 약한 매칭은 자동 입력 금지

비밀번호 매니저들의 자동채우기 도메인 매칭에서 배울 점을 요약하면 **"매칭 강도에 따라 가능한 행동이 달라진다"**는 것이다.

```text
1. exact origin 일치 (scheme + host + port)        → 자동 입력 허용
2. 명시적 제휴(affiliation) 일치                    → 제안만, 사용자 선택 필요
3. eTLD+1(등록 가능 도메인) 일치                    → 제안만, 자동 입력 금지
4. 유사도·브랜드·타이틀·파비콘 기반                 → 절대 금지
```

근거:

- W3C Credential Management 사양은 자격증명을 특정 origin에 유효한 것으로 규정하며, 초기 알고리즘은 교차 출처 누출을 막기 위해 eTLD+1 매칭이 아닌 **정확한 origin 매칭**을 요구했다. 출처: <https://www.w3.org/TR/credential-management-1/>
- Chromium 비밀번호 매니저는 exact / public-suffix / affiliation 매칭을 구분해 우선순위를 두고, public-suffix 매칭을 exact와 동등하게 취급하지 않으며, 교차 출처 iframe에서는 사용자가 계정을 선택하기 전까지 채우기를 미룬다. 출처: <https://chromium.googlesource.com/chromium/src/+/refs/heads/main/components/password_manager/core/browser/password_form_filling.cc>
- Apple의 password-manager-resources는 `password-rules.json`에서 `a.example.com`이 `b.example.com`과 매칭되지 않음을 명시하고(하위 도메인 상속은 같은 접두사 안에서만), `exact-domain-match-only: true` 플래그를 제공한다. `change-password-URLs.json`에는 하위 도메인별로 다른 항목이 존재해(예: `app.parkmobile.io` ≠ `parkmobile.io`) 키를 eTLD+1로 축소하면 안 된다. 출처: <https://github.com/apple/password-manager-resources>

### 3.2 흔한 구현 버그와 금지 목록

- **접미사 비교 금지**: `host.endsWith("example.com")`은 `notexample.com`, `evil-example.com`을 통과시킨다. 반드시 `host == key || host.ends_with("." + key)` 형태의 DNS 라벨 경계 검사를 쓴다.
- **http/https 구분**: 저장 origin이 https면 http 페이지에 자동 입력하지 않는다.
- **포트·스킴 포함 정규화**: origin 비교 전 scheme/host/effective port를 정규화한다(유니코드/punycode 혼동 주의).
- **iframe은 프레임 origin 기준**: 폼이 들어 있는 프레임의 origin이 자격증명 origin과 일치해야 한다. 최상위 페이지만 보고 iframe에 채우면 `attacker.example`이 `login.example`을 삽입하는 공격에 당한다.
- **리다이렉트 추적**: 로그인 흐름은 리다이렉트를 탄다. 최초 동의 origin에서 벗어나는 리다이렉트(특히 외부 도메인으로)는 자격증명 사용 중단 트리거다. OAuth의 `redirect_uri` 검증과 같은 원리.

### 3.3 근본 대책은 패스키

- 패스키(WebAuthn)는 비밀번호와 달리 **비밀값이 페이지로 전달되지 않고** RP ID에 암호학적으로 바인딩되어 look-alike 사이트에서 정상 호출 자체가 불가능하다. 출처: <https://docs.github.com/en/authentication/authenticating-with-a-passkey/about-passkeys>
- OxiBrowser 입장에서: 당장은 무리더라도, 자격증명 브로커의 저장 포맷에 `kind: password | passkey` 필드를 두고 패스키를 지원하는 사이트(Cloudflare, Tailscale, GitHub 모두 지원)에서는 패스키 경로를 우선 노출하는 설계가 장기적으로 피싱 면역성을 만든다.
- 단, 패스키의 RP ID는 보통 eTLD+1이라 **비밀번호의 exact-origin 규칙을 패스키에 적용하거나, 그 반대를 하면 안 된다**. 두 모델은 스코프 단위가 다르다.

---

## 4. 시크릿 리랙션 — 트랜스크립트/로그/HAR/스크린샷

### 4.1 HAR는 가장 큰 유출 면

HAR 파일은 쿠키(세션 쿠키 포함), `Authorization: Bearer …`, CSRF 토큰, 요청 바디의 비밀번호, URL 쿼리의 토큰(`access_token=`, `code=`)을 그대로 담는다. Chrome은 DevTools에서 **"Export HAR (sanitized)"**를 기본 제공해 `Cookie`/`Set-Cookie`/`Authorization` 헤더를 생략하며, 자동 살균이 전부를 잡지 못할 수 있음을 명시한다. 출처: <https://developer.chrome.com/docs/devtools/network/reference>, <https://support.auth0.com/center/s/article/How-to-Remove-Secrets-from-a-har-File>

리랙션 최소 목록(HAR 기준, OxiBrowser HAR에 그대로 적용):

```text
헤더: authorization, proxy-authorization, cookie, set-cookie,
      x-api-key, x-auth-token, x-access-token, x-refresh-token,
      x-csrf-token, x-xsrf-token  (+ 조직 고유 인증 헤더는 설정으로 추가)
쿠키: request.cookies[].value, response.cookies[].value
URL:  token, access_token, refresh_token, id_token, api_key, key,
      secret, signature, sig, code, state, password, email 파라미터 값
바디: request.postData, response.content 중 민감 필드
      (기본 전부 REDACTED, 디버그 필요 시 화이트리스트만)
```

### 4.2 에이전트 트랜스크립트/컨텍스트 리랙션

Operator/ChatGPT 에이전트가 취하는 접근이 기준점이 된다: **민감 정보 입력 단계에서는 에이전트가 스크린샷을 캡처하지 않고, 로그인/결제/CAPTCHA는 사용자가 브라우저를 직접 조작하는 takeover 모드로 넘긴다.** 출처: <https://openai.com/index/introducing-operator/>, <https://help.openai.com/en/articles/11752874-chatgpt-agent>

무인 재사용 설계에서는 입력이 에이전트가 하므로, 대신 다음을 강제한다:

1. **값 직접 노출 금지**: 비밀번호는 DOM 삽입 경로(§8의 브로커)로만 흐르고, LLM 컨텍스트·CDP 명령 로그·HAR에는 절대 평문으로 등장하지 않는다. 로그에는 `"password": "<redacted:sha256:ab12cd34>"` 형태의 핑거프린트만.
2. **입력 역리랙션(inverse redaction) 방지**: 자동채우기된 필드를 페이지 스크래핑 결과에서 다시 읽어오는 경로를 차단한다. `<input type=password>`의 value는 DomSnapshot/OXI.getMarkdown 출력에서 마스킹.
3. **스크린샷 마스킹**: 스크린샷에서 비밀번호 필드·OTP 입력·이메일 주소 등은 픽셀 블록 처리하거나, 민감 필드가 포커스된 순간은 캡처를 중단한다(Operator 방식의 자동화 버전).
4. **캡처된 토큰 격리**: 네트워크에서 OAuth 코드/토큰이 관측되면 즉시 브로커 금고로 옮기고 로그에는 토큰 타입과 해시만 남긴다.

---

## 5. 되돌릴 수 없는 행동의 확인 정책

### 5.1 확인은 두 층으로 분리

OpenAI Operator의 설계에서 배울 점: **"민감 정보 입력을 위한接管(takeover) 확인"과 "행동 실행 승인"은 별개의 통제**다. 반환/인계 자체가 결제 승인을 의미하지 않는다. 또한 결제 직전에는 판매자·금액·수령인·지불 수단을 담은 **구조화된 요약**으로 승인을 받고, **재료(가격·수신자·수단)가 바뀌면 이전 승인을 무효화**한다. 출처: <https://openai.com/index/introducing-chatgpt-agent/>, <https://openai.com/index/introducing-operator/>

에이전트 프레임의 확인 정책으로 일반화:

```text
불가역 행동 분류(기본 태그):
  - 금전: 결제, 구독 변경, 송금
  - 파괴: 삭제, 드롭, 해제(터널/노드 삭제 포함)
  - 외부 전송: 이메일/메시지 발송, 공개 게시, 초대
  - 인증 상태 변경: 로그아웃(iCloud 등), 비밀번호/2FA 변경, 세션 전체 폐기
  - 권한 부여: API 키 발급, OAuth 동의 화면 승인, 관리자 초대

확인 프로토콜:
  1. 행동 실행 직전에 요약(대상 origin, 정확한 값, 되돌림 가능 여부) 생성
  2. 사용자 명시 승인 대기 — 타임아웃 있음, 기본 거부
  3. 승인 후 실행 직전 재검증: 요약 재계산, 값이 바뀌었으면 승인 무효
  4. "확인"은 결코 페이지 콘텐츠가 아닌 에이전트 프레임이 렌더
```

### 5.2 승인 피로에 대한 현실적 방어

- CSA GhostApproval 등의 연구는 "confirm before acting"이 UI 순간에 불과하면 실패함을 보여준다. 출처: <https://labs.cloudsecurityalliance.org>
- 따라서: (a) 확인 대상은 **행동 클래스별로 결정론적으로 트리거**(모델의 판단에 맡기지 않음), (b) 승인 프롬프트 빈도를 낮추기 위해 안전한 행동(읽기, 탐색)은 동의 없이 허용, (c) 예산 상한(예: 세션당 결제 총액, 삭제 n건)을 두어 승인 하나당 폭발 반경 제한.
- 배경 사례 매핑: Tailscale 관리콘솔 승인·iCloud 로그아웃은 "권한 변경/인증 상태 변경" 클래스 → 자동화하더라도 항상 최종 확인 + 감사 로그. 시스템설정 GUI 승인(macOS TCC 프롬프트)은 OS 레벨이므로 에이전트가 대신 클릭하지 못하게 하는 게 원칙이며, 사전에 사용자가 미리 승인하거나 `tccutil` 기반 사전 구성으로 요구 자체를 제거한다.

### 5.3 확인 요청 자체의 인젝션 방어

페이지가 "결제를 승인하려면 이 버튼을 누르세요" 같은 지시를 포함할 수 있다. 규칙: **페이지 텍스트의 지시는 실행 근거가 될 수 없다.** 확인 카드에는 프레임이 계산한 origin·DOM 속성 기반 값만 표시하고, 페이지 내용 인용은 "신뢰 없는 인용"으로 명시 라벨링한다(Chrome 에이전트 보안 가이드와 동일). 출처: <https://developer.chrome.com/docs/agents/security>

---

## 6. 세션 폐기·만료·침해 대응

### 6.1 토큰·세션 수명 설계

OAuth 관점의 기본기(RFC 9700 BCP):

- 짧은 수명의 액세스 토큰 + 리프레시 토큰 회전(rotation) 또는 송신자 결합(sender-constrained) 토큰 둘 중 하나가 공개 클라이언트에 요구된다. 출처: <https://www.rfc-editor.org/info/rfc9700/>
- 폐기: RFC 7009 리보케이션 엔드포인트로 리프레시 토큰(필수 지원)과 액세스 토큰(권장)을 무효화한다. 다만 자기완결형(self-contained) 액세스 토큰은 즉시 무효화가 어려워, **짧은 만료가 실질적 방어**가 된다. 출처: <https://www.rfc-editor.org/info/rfc7009/>
- 쿠키 세션: 탈취 시 악용 가능한 bearer 비밀이다. 세션 종료 시 브라우저 프로필·쿠키 저장소를 폐기하고, 필요시 사이트 로그아웃 엔드포인트를 호출해 서버 측 세션까지 무효화한다. Operator 문서도 "쿠키가 세션 간 지속될 수 있으니 로그아웃/데이터 삭제 수단을 제공하라"고 명시한다. 출처: <https://help.openai.com/en/articles/11752874-chatgpt-agent>

### 6.2 에이전트 세션 수명 주기 권고

```text
세션 시작:  임시 쿠키 저장소(세션 스코프), 필요한 자격증명만 동의 로드
세션 중:    하트비트 + 비활동 타임아웃(예: 30분) → 자격증명 언로드
세션 종료:  1) 사이트 로그아웃(가능 시) 2) 쿠키/스토리지 파기
            3) 캡처 토큰 리보케이션 4) 감사 로그 봉인(append-only)
침해 의심:  1) 해당 origin 동의 전부 즉시 revoke
            2) 키체인에서 해당 항목 폐기/교체(비밀번호 변경)
            3) 영향받은 사이트 세션 전체 로그아웃
            4) 감사 로그 기반 사용 이력 사용자 통지
```

### 6.3 DPoP/MTLS로 탈취 대비

토큰을 애초에 송신자에게 결합하면 유출되어도 재생이 어렵다. DPoP는 클라이언트 개인키 서명 증명을 요구해 "토큰 소지"만으로는 API 호출이 불가하게 만든다. 에이전트가 API 토큰을 직접 다루는 경로(예: 캡처한 Tailscale/Cloudflare API 토큰)에 적용을 검토한다. 출처: <https://www.rfc-editor.org/info/rfc9449/>

---

## 7. OAuth 스코프와 캡처 토큰의 최소 권한

### 7.1 원칙

- 스코프는 "토큰이 **무엇을** 할 수 있는가", DPoP는 "**누가** 쓸 수 있는가", 폐기는 "**얼마나 오래** 쓸 수 있는가"를 각각 통제한다(§6). 세 축을 함께 설계한다. 출처: <https://www.rfc-editor.org/info/rfc9700/>
- 최신 BCP는 스코프 외에 **audience(대상 리소스 서버), 리소스, 행동(read vs write)** 제한을 권고한다. `orders.read` 하나가 사용자 A/B 구분을 대신하지 못하므로 리소스 서버 쪽 인가는 여전히 필요하다.
- MCP의 인증 가이드도 같은 방향이다: 클라이언트는 OAuth 2.0 모범 사례에 따라 토큰을 안전하게 보관해야 하고(MUST), 서버는 만료·회전을 강제해야 한다(SHOULD). 출처: <https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/authorization>

### 7.2 "캡처된 토큰"의 특수성 — 브라우저 세션에서 추출한 토큰

배경 사례(Cloudflare 터널 인증, Tailscale 콘솔)에서 에이전트가 얻는 것은 대개 **전체 사용자 권한의 세션 쿠키/토큰**이다. 이는 태생적으로 과권한이다:

1. **다운스코프 시도**: 가능하면 브라우저 세션 대신 사이트가 제공하는 범위 좁은 API 토큰 경로를 우선 사용한다. 예: Cloudflare 터널은 `cloudflared` 전용 인증 흐름(`cloudflared tunnel login`, Origin CA 키)이 있고, Tailscale은 OAuth 클라이언트/태그 기반 스코프 토큰을 제공한다 — 콘솔 세션 전체가 아니라 "터널 생성" 권한만 주는 토큰이 존재한다.
2. **캡처 토큰 격리**: 어쩔 수 없이 세션 토큰을 캡처했으면 (a) 메모리/금고 보관, (b) 세션 종료 시 폐기(§6), (c) HAR·로그 리랙션 대상(§4), (d) 가능하면 즉시 다운스코프된 토큰으로 교환 후 원본 폐기.
3. **OAuth 화면 자동 승인 금지**: 동의 화면(scope 목록 표시)은 §5의 "권한 부여" 불가역 클래스로 취급한다. 모델이 scope 텍스트를 읽고 판단하는 게 아니라, 프레임이 파싱한 scope 목록을 사용자에게 그대로 보여준다.
4. **redirect_uri 고정**: 브로커가 OAuth 코드 플로를 대행할 때 redirect_uri는 `localhost` 고정 + state/PKCE 필수. PKCE는 공개 클라이언트의 인터셉션 방어 기본(RFC 9700).

---

## 8. OxiBrowser 통합 지점 제안

현재 구조(docs/ARCHITECTURE.md, docs/CDP.md, 소스 확인 기반)에 대응시킨 구체 지점. 핵심 아이디어는 **CDP/LLM이 자격증명 값에 접근하는 유일한 경로를 브로커로 일원화**하는 것이다 — Chrome이 CDP를 "브라우저 침해와 동등"으로 취급하고 전용 프로필+로컬 바인딩을 요구하는 것과 같은 이유. 출처: <https://developer.chrome.com/blog/remote-debugging-port>

### 8.1 자격증명 브로커 크레이트 (`oxibrowser-credentials`, 신규)

```text
역할:
  - 키체인/비밀 저장소 어댑터 (macOS: security-framework crate)
  - 자격증명 핸들 발급: CredentialId -> (origin 화이트리스트, actions, 만료)
  - 값은 Zeroize + 메모리 보관, 절대 Serialize/Debug 노출 금지
  - 감사 로그(JSONL append-only) 기록

macOS 키체인 연계:
  - 항목별 ACL로 "신뢰 앱"을 지정할 수 있다(레거시 키체인, -T 옵션).
    에이전트 바이너리가 직접 키체인 열람 권한을 갖게 하지 말고,
    브로커 프로세스만 항목별로 신뢰 등록. 출처:
    https://developer.apple.com/documentation/security/access-control-lists
  - 현대 키체인은 access group + 코드서명 기반이므로, 브로커를 별도
    서명된 바이너리로 분리하면 그룹 기반 격리도 가능.
```

### 8.2 Origin 정책 모듈 (`oxibrowser-core/src/network/origin_policy.rs`, 신규 — `ip_filter.rs` 옆)

- `IpFilter`가 SSRF CIDR를 차단하듯, `OriginPolicy`가 **자격증명 주입 허용 origin 목록**을 강제한다.
- exact origin 매칭(§3.1 계층: exact 자동 / affiliation·eTLD+1 제안만) + DNS 라벨 경계 검사 + iframe 프레임 origin 검사.
- `HttpClient`가 요청을 보낼 때와 로그인 폼 감지 시점에 정책을 평가한다. 리다이렉트로 origin이 바뀌면 자격증명 세션 무효화 이벤트 발행.

### 8.3 HAR 리랙션 게이트 (`session.rs` `network_log` → `main.rs` `write_har` 경로)

- 현재 `RequestRecord`는 `request_headers`, `post_body`, `response_headers`를 그대로 담고 `--har`로 덤프한다 → 오늘 기준 Cookie/Authorization이 평문으로 HAR에 기록된다(소스 확인: `crates/oxibrowser-core/src/session.rs` L39–64, `crates/oxibrowser/src/main.rs` `write_har`).
- 권고: `network_log_har_json()` 직전에 `redact::har()`를 강제 통과(§4.1 목록). 원본이 필요한 디버그는 `--har-raw` 플래그로 명시적 옵트인 + 경고 출력. CDP 이벤트(`Network.requestWillBeSent`)로 브로드캐스트되는 헤더도 같은 필터를 적용한다.

### 8.4 쿠키 jar 세션 스코프화 (`browser.rs` 글로벌 `cookie_jar`)

- 현재 `CookieJar`는 Browser 글로벌이고 `cookie_file`로 디스크 지속도 된다(소스 확인: `browser.rs` L50, L69–89). 자격증명 재사용 시나리오에서는 세션 간 세션 누출이 위험.
- 권고: 세션별 jar(또는 jar 네임스페이스) + 세션 teardown 시 `CookieJar::clear()`(이미 존재, `cookie.rs` L677) + 파일 지속은 명시적 옵트인. CDP `Network.getAllCookies`는 자격증명 세션에서 기본 거부(§8.5).

### 8.5 CDP 도메인 게이팅 (`oxibrowser-cdp/src/domains/`)

- 크레딴셜 모드에서 위험한 CDP 메서드를 게이트: `Network.getAllCookies`, `Network.getCookies`, `Runtime.evaluate`(문서 쿠키 읽기 `document.cookie` 포함), `Input.insertText`(비밀번호 입력은 브로커 전용 경로로만). Chrome 에이전트 가이드가 권고하는 "모델에 무제한 CDP 대신 좁은 도구 API" 원칙. 출처: <https://developer.chrome.com/docs/agents/security>
- 새 이벤트: `OXI.confirmationRequired { action, origin, summary }` → 클라이언트(오케스트레이터)가 `OXI.resolveConfirmation`으로 승인/거부. §5 확인 프로토콜의 CDP 표면.

### 8.6 세션 executor 정책 훅 (`oxibrowser/src/session/executor.rs`)

- 22개 파서 명령이 Tab 메서드로 매핑되는 지점이 유일한 관문이다. 명령 실행 전 인터셉터 체인에 `PolicyEngine` 삽입:
  ```text
  명령 → deny 규칙 → 동의 캐시 확인 → (필요 시) confirmation 이벤트 대기 → 실행 → 감사 로그
  ```
- 파서(`parser.rs`)에 새 명령 추가: `credential_list`, `credential_authorize`(동의), `credential_use(origin, id)`. 값 자체는 명령 인자로 못 오게 타입 레벨에서 봉쇄.

### 8.7 OXI 도메인 확장 (`domains/oxi.rs`)

- `OXI.getMarkdown`/`OXI.getPageInfo` 출력에 리랙션 라벨 적용: `type=password` 입력값 마스킹, 감지된 토큰 형태(JWT `eyJ…`, `Bearer …`) 자동 마스킹 + `UNTRUSTED_PAGE_CONTENT` 메타 표시(§4.2, §5.3).

### 8.8 스킬 시스템 연계 (`skills/`, `skill.rs`)

- 스킬 매니페스트에 `requires: { credentials: [...], irreversible_actions: [...] }` 선언 필드. 스킬 로드 시 프레임이 이 선언을 사용자 동의 요청과 매칭(§2). 스킬 문서(마크다운)에 포함된 지시는 권한 근거로 취급하지 않는다.

---

## 9. 실행 가능한 권고

우선순위 순. P0는 유출 방어(지금 구조의 실제 취약점), P1은 무인 자격증명 재사용의 전제 조건, P2는 성숙도 확보.

**P0 — 즉시 (유출 면 축소)**
1. HAR/네트워크 로그 리랙션(§8.3): `Authorization`/`Cookie`/`Set-Cookie` 헤더와 민감 쿼리 파라미터를 기본 REDACT. 원본은 `--har-raw` 명시적 옵트인으로.
2. 스크린샷/DomSnapshot에서 `type=password` 값 마스킹(§4.2).
3. `cookie_file` 디스크 지속을 기본 끔, 세션 종료 시 jar clear(§8.4).
4. 감사 로그 JSONL(credential 접근·민감 행동·정책 위반) 도입 — 값 대신 핸들+해시(§2.2).

**P1 — 자격증명 재사용 전제 (피싱·오용 방어)**
5. `origin_policy.rs` exact-origin 매칭 + DNS 라벨 경계 검사 + 프레임 origin 검사(§3, §8.2). 유사도/브랜드 매칭은 구현 금지 목록에 명문화.
6. 자격증명 브로커 프로세스 분리 + macOS 키체인 항목별 ACL 등록(§8.1). 비밀값의 LLM/CDP 경로 노출 원천 차단.
7. 동의 레코드(credential × origin × action, 만료·횟수 제한)와 deny-우선 정책 엔진(§2.1), executor 인터셉터 체인(§8.6).
8. 불가역 행동 분류·확인 프로토콜·승인 무효화 조건(§5). iCloud 로그아웃·관리콘솔 승인·OAuth 동의 화면은 항상 확인 클래스.
9. OAuth 동의 화면 자동 승인 금지, redirect_uri 고정 + PKCE(§7.2).

**P2 — 성숙도 (수명·침해 대응)**
10. 세션 타임아웃·하트비트, 종료 시 사이트 로그아웃 + 토큰 리보케이션(RFC 7009) 시퀀스(§6.2).
11. 캡처 토큰의 다운스코프 우선 정책: Cloudflare는 `cloudflared` 전용 인증/Origin CA, Tailscale은 스코프 OAuth 토큰 경로 우선(§7.2).
12. 패스키 저장 포맷 필드(`kind: password|passkey`) 확보(§3.3).
13. 침해 대응 플레이북 문서화 + 감사 로그 기반 사용 이력 통지 흐름(§6.2).

---

## 10. 주요 출처

1. W3C Credential Management Level 1 — 자격증명의 origin 유효 범위와 exact-origin 매칭: <https://www.w3.org/TR/credential-management-1/>
2. Chromium password_manager `password_form_filling.cc` — exact/PSL/affiliation 매칭 계층과 iframe 지연 채우기: <https://chromium.googlesource.com/chromium/src/+/refs/heads/main/components/password_manager/core/browser/password_form_filling.cc>
3. Apple password-manager-resources — 도메인 매칭 규칙·`exact-domain-match-only`·change-password URL 데이터: <https://github.com/apple/password-manager-resources>
4. GitHub Docs — 패스키의 RP ID 바인딩과 피싱 저항: <https://docs.github.com/en/authentication/authenticating-with-a-passkey/about-passkeys>
5. Apple Developer — Keychain 항목 ACL(신뢰 앱) 모델: <https://developer.apple.com/documentation/security/access-control-lists>
6. Chrome DevTools Network Reference — sanitized HAR 내보내기와 민감 헤더 제거: <https://developer.chrome.com/docs/devtools/network/reference>
7. Auth0 — HAR에서 시크릿 제거 가이드(유출 시 대응 포함): <https://support.auth0.com/center/s/article/How-to-Remove-Secrets-from-a-har-File>
8. OpenAI — Introducing Operator (takeover mode, 민감 입력 중 캡처 중단): <https://openai.com/index/introducing-operator/>
9. OpenAI Help — ChatGPT agent (로그인/결제 입력 규칙, 쿠키 지속·로그아웃): <https://help.openai.com/en/articles/11752874-chatgpt-agent>
10. OpenAI — Introducing ChatGPT agent (불가역 행동 확인 원칙): <https://openai.com/index/introducing-chatgpt-agent/>
11. Anthropic — Claude Code IAM (deny 우선 권한 규칙·모드): <https://docs.anthropic.com/en/docs/claude-code/iam>
12. RFC 9700 — OAuth 2.0 Security BCP (audience·스코프 최소화, 회전/송신자 결합 요구): <https://www.rfc-editor.org/info/rfc9700/>
13. RFC 9449 — DPoP (송신자 결합 토큰): <https://www.rfc-editor.org/info/rfc9449/>
14. RFC 7009 — OAuth 2.0 Token Revocation: <https://www.rfc-editor.org/info/rfc7009/>
15. Chrome for Developers — Agent security considerations (신뢰 불가 페이지 콘텐츠, 도구 출력 오염): <https://developer.chrome.com/docs/agents/security>

부록 참고: Chrome remote debugging 변경(전용 user-data-dir 요구, CDP=브라우저 침해 취급) <https://developer.chrome.com/blog/remote-debugging-port> · MCP 인증 보안 가이드 <https://modelcontextprotocol.io/docs/2026-07-28/tutorials/security/authorization> · CSA GhostApproval(승인 피로) <https://labs.cloudsecurityalliance.org>
