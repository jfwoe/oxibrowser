# OxiBrowser 에이전트 무인 인증 — 연구 종합 (2026-09-27)

> 원 질문: "오늘 서버 세팅에서 내가 직접 계정에 접속해야 했던 단계들(Cloudflare 터널 인증, Tailscale 콘솔 승인, iCloud 로그아웃, 시스템설정 GUI 승인)을, OxiBrowser가 사용자 자격증명(키체인 등)을 저장해 알아서 진행하게 하려면?"
>
> 조사: herdr 서브에이전트 6개 병렬 (01~06 문서). 본 문서는 종합·우선순위화.

## 1. 핵심 결론 — 4개 막힘 지점의 운명

| 지점 | 무인화 가능성 | 정답 |
|---|---|---|
| Cloudflare 터널 인증 | ✅ **100%** | 브라우저 필요 없음. API 토큰(Account: Tunnel Edit + Zone: DNS Edit) 1회 발급 → `POST /cfd_tunnel` 시퀀스. `cloudflared tunnel login`은 폐기 대상 (03) |
| Tailscale 콘솔 승인 | ✅ **100%** | OAuth 클라이언트 + `autoApprovers` 정책(태그 기반 라우트 자동승인) + `POST /device/{id}/routes` API. 콘솔 클릭 자체가 필요 없어짐 (03) |
| iCloud 로그아웃 | ❌ **구조적 불가** | Apple이 무인 API를 제공하지 않음(MDM 명령 카탈로그에 없음). `defaults delete MobileMeAccounts`는 불완전 상태를 만드는 금지 경로. **사용자 확인 게이트로 명시적 분리가 올바른 설계** (03) |
| 시스템설정 GUI 승인 | ⚠️ 조건부 | 관리 기기면 MDM SystemExtensions/PPPC 사전 승인. 개인 Mac이면 코드서명된 privileged helper로 "1회 인증"으로 압축하거나 게이트 (03) |

**가장 큰 통찰**: 오늘 막힌 4개 중 2개는 브라우저조차 필요 없는 **API-퍼스트** 경로가 존재했다. "에이전트가 브라우저로 로그인하게 한다"는 2순위 전략이고, 1순위는 **스코프 축소된 토큰의 사전 프로비저닝**이다.

## 2. 4층 전략 스택 (결정트리)

```
Q1. 문서화된 API가 있는가? ── YES → API 토큰 경로 (스코프·TTL·IP 제한, 브로커 보관)
Q2. 웹 세션으로만 되는가? ── YES → 세션 지속성 (storage_state + 지문·egress 동반, 도메인 스코프 암호화 저장)
Q3. 로그인 폼/2FA가 필요한가? ── YES → 자격증명 브로커 (키체인 → 실행 직전 주입 + TOTP/가상 패스키)
Q4. SMS/이메일 2FA, OS GUI, iCloud 계정 조작? ── 인간 상승 (takeover 게이트, 코드는 에이전트 경유 금지)
```

## 3. 무인 자격증명 재사용의 3대 원칙 (전 제품·문서 수렴)

1. **비밀값은 절대 LLM 컨텍스트/프롬프트/로그로 흐르지 않는다.** Operator(secure form·인계 중 미캡처), Steel(Credentials API 주입), browser-use(`<secret>` 치환), Stagehand(variables), Skyvern(저장소 조회) — 전부 동일. 구현 차이는 "누가 채우는가"뿐.
2. **인증 상태는 2층**: ① 브라우저 상태(쿠키·스토리지) 영속 — 세션 저장소/프로파일, ② 원천 자격증명(비밀번호·TOTP 시크릿) 보관 — 키체인/볼트. Steel의 "credential으로 수립 → profile로 지속 → 만료 시 credential으로 복구" 플로우가 정석.
3. **확인 게이트는 실행 계층에서 결정론적으로.** LLM에게 "물어보라"고 시키는 방식(browser-use ask_human)은 모델이 우회 가능 — 반면교사. deny → allow → 프롬프트 순서 평가(Claude Code IAM 모델), 동의는 credential × exact origin × action 단위 + 만료·횟수 제한.

## 4. OxiBrowser 현황 대비 갭

**이미 있는 것** (이번 조사로 확인):
- `storage_state.rs` — Playwright 호환 포맷(cookies + origins[].localStorage) 그대로 미러링. 단 **export가 단일 오리진 한정** → 다중 오리진 스코프 확장 필요 (02)
- `challenge.rs` — Cloudflare/DataDome/PerimeterX 분류 + Interactive/Blocked 중단 조건 = **인간 상승 감지 계층 이미 존재**. 에이전트 워크플로 이벤트로 연결만 하면 됨 (02)
- `js/stealth.rs` + pure-rust-stealth 설계문서 — 지문 일관성 작업 진행 중 (02)
- `CookieJar`(network/cookie.rs) — sameSite/httpOnly/expiry/partitioned/partition_key 직렬화 이미 지원. 단 Browser **글로벌** jar + 디스크 지속 → 세션 스코프화 필요 (06)
- `skills/` + `oxibrowser skill` — install·webfetch 스킬 존재 → auth 스킬 슬롯 비어있음
- session REPL 22명령 + `mcp.rs` — executor 관문이 명확해서 정책 인터셉터 삽입 지점 깔끔 (06)

**없는 것 (신규 필요)**:
- HAR 리랙션 — 현재 `--har`가 Cookie/Authorization 평문 기록 (06 §8.3, P0)
- WebAuthn 전무 — `grep webauthn|navigator.credentials` 무일치 (04 §1.4)
- 자격증명 브로커 · origin 정책 · 동의/감사 · takeover 모드 · TOTP 생성

## 5. 우선순위 로드맵 (6편 권고 통합)

**P0 — 유출 면 축소 (지금 구조의 실제 취약점, 자격증명 기능과 무관하게 시급)**
1. HAR/네트워크 로그 리랙션 기본값 (Authorization/Cookie/Set-Cookie/민감 쿼리 REDACT, `--har-raw` 명시적 옵트인)
2. DomSnapshot·스크린샷에서 `type=password` 마스킹
3. `cookie_file` 디스크 지속 기본 끔 + 세션 종료 jar clear
4. 감사 로그 JSONL (값 대신 핸들+해시)

**P1 — 무인 자격증명 재사용의 전제**
5. `origin_policy.rs` — exact origin 매칭(DNS 라벨 경계 검사, iframe 프레임 origin 기준) + 리다이렉트 origin 이탈 시 자격증명 세션 무효화
6. 자격증명 브로커 (`oxibrowser-credentials`) — 키체인 어댑터(keyring/security-framework) + 핸들 발급 + Zeroize + 봉쇄: 값이 CDP 명령 인자·로그로 못 나감. 저장 포맷은 JSON 레코드(otpauth:// URI 정규화 포함), 서비스 키 `com.oxibrowser.agent/<agent>/<site>`
7. TOTP 생성 내장 (hmac+sha1+base32 ~30줄 또는 totp-rs) — Cloudflare/Tailscale 2FA 무인화의 최저비용 조각
8. executor 정책 인터셉터 + 동의 레코드(만료·횟수 제한) + `OXI.confirmationRequired` CDP 이벤트
9. takeover 모드 — 입력 채널 사용자 독점 위임 + 그 구간 캡처 중단 (Operator UX 복제, CDP 기반이라 구현 저렴)
10. 세션 저장소 봉투 — 지문·egress 메타 동반 + AEAD 암호화(키체인에 키) + 도메인 스코프 파일 + 다중 오리진 export

**P2 — 성숙도**
11. WebAuthn 가상 인증기 (CDP WebAuthn 도메인 + boa에 navigator.credentials, ES256 p256) — "사용자가 1회 로그인해 에이전트용 패스키 등록 → 이후 무인 서명". iCloud 기존 패스키 재사용은 불가이므로 시도 금지
12. 프로파일 2층 구조 (영구 프로파일 + persist:false 읽기전용 + advisory lock — Skyvern #4390 반면교사)
13. 도메인 허용목록 + 스킬 매니페스트 `requires: {credentials, irreversible_actions}` 선언

## 6. 명시적 비권고 (조사에서 근거 확보된 금지 목록)

- `cf_clearance` 스냅샷 재생 의존 — 방문자·디바이스 결속으로 재사용 불신뢰 (02 §3.2)
- 사용자 일상 Chrome 프로파일 attach — Chrome 136부터 원천 차단 + 최상위 위험 (02 §2.2)
- SMS/이메일 2FA 자동화 — 구조가 피싱 릴레이와 동일 (04 §4.1, NIST restricted)
- iCloud 기존 패스키 외부 사용 — 비밀키 비노출·연관도메인 필요로 불가 (04 §3)
- `defaults delete MobileMeAccounts`식 iCloud 우회 — 불완전 상태 (03 §5.1)
- LLM 프롬프트 기반 승인 요청 — 우회 가능, 실행 계층 게이트만 유효 (05 §2.3)
- 브랜드·타이틀·유사도 기반 자격증명 매칭 — password-manager-resources가 반증 (06 §3.1)

## 7. 오늘 사례에 즉시 적용할 수 있는 것 (oxibrowser 개발 없이)

- Cloudflare: API 토큰 1회 발급 → 터널 전 과정 API화 (03 §3.2 시퀀스 그대로)
- Tailscale: OAuth 클라이언트 + autoApprovers 정책 배포 → 라우트 승인 클릭 영구 제거
- TOTP 시크릿 키체인 저장 + `oathtool -b --totp "$(security find-generic-password ...)"` 파이프라인
- 이 3개만으로 오늘의 "브라우저 2개" 수동 단계가 사라지고, 남은 것은 구조적으로 인간 게이트가 맞는 2개

## 문서 인덱스

| 파일 | 주제 |
|---|---|
| 01-keychain.md | macOS 키체인 구조(TN3137), ACL/partition ID, Rust 경로, 브로커 비교, 저장 포맷 |
| 02-session-persistence.md | storageState/프로파일, 쿠키 파티셔닝, 봇 탐지·완화, 세션 저장소 설계안 |
| 03-api-first-matrix.md | CF/TS/Apple 무인 경로 매트릭스, 토큰 스코프·회전, 결정트리 |
| 04-2fa-passkeys.md | WebAuthn 가상 인증기, TOTP, iCloud 패스키 제약, 인간 상승 설계, step-up |
| 05-landscape.md | Operator/computer-use/browser-use/Steel/Browserbase/Stagehand/Skyvern 비교, 복사할 패턴 Top 5 |
| 06-safety-design.md | 위협 모델, 동의·감사, origin 고정, 리랙션, 확인 프로토콜, 세션 수명, OxiBrowser 통합 지점 8곳 |
