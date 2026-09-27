# 세션 지속성 — 에이전트의 인증 세션 재사용 조사

> 조사 배경: 서버 세팅 에이전트가 Cloudflare 터널 인증(브라우저), Tailscale 관리콘솔 승인, macOS iCloud 로그아웃, 시스템설정 GUI 승인 단계에서 사용자 손을 빌려야 했음. 목표는 사용자 자격증명(키체인 등)을 안전히 재사용해 무인으로 진행하는 것. 이 문서는 그중 **브라우저 인증 세션 지속성**을 다룬다.
>
> 대상 코드: OxiBrowser v0.22 (`~/Documents/Workspace/Projects/oxibrowser`) — `crates/oxibrowser-core/src/storage_state.rs`, `network/cookie.rs`, `session.rs`, `challenge.rs`, `js/stealth.rs`, `crates/oxibrowser-cdp/`.

---

## 1. 인증 세션 지속 기법

### 1.1 Playwright `storageState` — 사실상의 표준 포맷

Playwright는 "한 번 로그인 → 상태 저장 → 이후 컨텍스트에 주입" 패턴을 표준화했다. 저장 단위는 **쿠키 전체 + origin 단위 localStorage**이며 JSON으로 직렬화된다(`{cookies: [...], origins: [{origin, localStorage: [{name, value}]}]}`). 코드젠(`playwright codegen --save-storage/--load-storage`)으로 인간이 수동 로그인한 뒤 상태를 뽑는 워크플로도 공식 지원한다. 출처: [Playwright Authentication](https://playwright.dev/docs/auth), [Playwright Codegen](https://playwright.dev/docs/codegen)

핵심 유의점:

- **sessionStorage는 저장되지 않는다.** 앱이 토큰을 sessionStorage에 두면 수동으로 export한 뒤 `addInitScript`로 복원해야 한다. ([Playwright Auth](https://playwright.dev/docs/auth))
- **인증 완료 확인 후 저장해야 한다.** 로그인 리다이렉트 레이스를 피하기 위해 dashboard URL 등 인증된 상태의 신호를 기다린 뒤 `storageState()` 호출.
- 스냅샷 파일 자체가 **계정 탈취 가능한 bearer credential**이므로 절대 VCS에 커밋하지 않는다. ([Playwright Auth](https://playwright.dev/docs/auth))
- 최신 Playwright는 IndexedDB 기반 인증·WebAuthn/패스키 상태도 언급하지만, 쿠키+localStorage가 여전히 실무 표준. ([Playwright Auth](https://playwright.dev/docs/auth))
- 상태는 도메인 매칭되는 쿠키/오리진에만 적용되고, 만료 시 재생성 필요. (같은 문서의 Common issues)

**OxiBrowser 현황**: `storage_state.rs`의 `StorageState`가 이 포맷을 정확히 미러링한다(`cookies` + `origins[].localStorage`, Playwright 철자 그대로). `Session::export_state`/`import_state`로 주고받고, import된 localStorage는 `pending_seed`로 다음 문서 로드 시 JS 런타임에 주입된다(`session.rs:262-266, 1826-1830`). 단 **export가 현재 페이지 오리진 하나로 한정**된다(`session.rs:2260` 부근, 테스트가 "single-origin limitation"을 명시). 세션 저장소 설계(§4)에서 이 제약이 핵심 개선 지점이다.

### 1.2 Chrome `user-data-dir` — 프로파일 통째 재사용

영구 프로파일 디렉터리로 실행하면 쿠키·localStorage·IndexedDB·서비스워커·확장까지 통째로 유지되어 "로그인이 유지되는 브라우저"가 된다. 가장 강력하지만 가장 무거운 방식. ([Chromium user_data_dir 문서](https://chromium.googlesource.com/chromium/src/%2B/HEAD/docs/user_data_dir.md))

유의점:

- **한 디렉터리 = 한 Chrome 프로세스.** 두 인스턴스가 같은 user-data-dir을 쓰면 프로파일 락·SQLite 경합·상태 유실·프로파일 손상이 생긴다. 동시 실행 시 디렉터리를 반드시 분리. (같은 문서)
- **버전·이식성 제약.** 프로파일은 Chrome/Chromium 버전 간·머신 간·NFS/SMB 마운트에서 하위 호환이 보장되지 않는다. ([Chromium 정책 문서](https://chromium.googlesource.com/website.git/%2B/HEAD/site/administrators/policy-list-3/user-data-directory-variables/index.md))
- **디렉터리 전체가 자격증명 금고.** 파일을 읽을 수 있는 프로세스는 로그인된 사용자로 행동할 수 있다. 0700 퍼미션, 전용 OS 사용자, 로컬 암호화 디스크, 백업/아티팩트/SCM 제외 필요. ([Chromium user_data_dir 문서](https://chromium.googlesource.com/chromium/src/%2B/HEAD/docs/user_data_dir.md))
- **프로파일 재사용 ≠ 영구 로그인.** 서버측 로그아웃·토큰 로테이션·IP/디바이스 변화·리스크 감지로 세션은 무효화될 수 있으니, 미인증 상태 감지→재로그인/중단 처리를 자동화에 포함해야 한다.
- **격리 기본값**: ephemeral한 브라우저 컨텍스트(TARGET/`Target.createBrowserContext`)가 대부분 작업에 더 안전하고, 재사용이 필요하면 "계정+워커당 1개 전용 프로파일"이 정석. ([CDP Target 도메인](https://chromedevtools.github.io/devtools-protocol/tot/Target/))

### 1.3 쿠키/스토리지 직렬화·복원 시 유의점

쿠키를 저장·복원할 때 놓치기 쉬운 속성들. CDP `Network.setCookies`/`Storage.setCookies`의 `CookieParam` 제약이 좋은 체크리스트다. ([CDP Storage](https://chromedevtools.github.io/devtools-protocol/tot/Storage/), [CDP Network](https://chromedevtools.github.io/devtools-protocol/tot/Network/))

| 속성 | 함정 | 올바른 처리 |
|---|---|---|
| `session` (세션 쿠키) | 읽기 전용 필드를 그대로 다시 set하려 하면 안 됨 | `expires`를 **생략**하면 세션 쿠키로 복원됨. 복사한 옛 `expires`가 이미 지나간 경우 유효하지 않음 |
| `sameSite` | `Unspecified` 값을 그대로 보내면 거부 | `Strict`/`Lax`/`None`만 전달. `SameSite=None`은 `Secure=true`와 세트 |
| `__Secure-` 접두사 | HTTP URL로 복원 시 실패 | 이름 유지 + `secure: true` + HTTPS `url`/`domain` 사용 |
| `__Host-` 접두사 | `domain` 포함 시 실패 | `secure: true`, `path: "/"`, domain 없이 HTTPS url로만 |
| `partitionKey` (CHIPS) | 생략하면 파티션되지 않은 *다른* 쿠키가 됨 | `topLevelSite`(schemeful site) + `hasCrossSiteAncestor`(Chrome 128+)까지 보존 |
| HttpOnly | JS(`document.cookie`)로는 읽을 수 없음 | export는 브라우저 내부 쿠키 저장소/디버깅 프로토콜 접근 필요. OxiBrowser는 자체 `CookieJar`를 직접 직렬화하므로 제약 없음 |

쿠키 파티셔닝 관련 배경:

- **CHIPS**: `Partitioned` 속성 쿠키는 최상위 사이트별 별도 쿠키 jar에 저장된다. 3rd-party 컨텍스트 인증(예: 임베드된 위젯 로그인)을 복원하려면 파티션 키까지 저장해야 같은 파티션에 복원된다. ([Privacy Sandbox CHIPS](https://privacysandbox.google.com/cookies/chips), [MDN CHIPS](https://developer.mozilla.org/en-US/docs/Web/Privacy/Partitioned_cookies))
- 실제로 Cloudflare가 `cf_clearance`에 `SameSite=None; Secure; Partitioned` 속성을 사용하므로(§3.2), Cloudflare 보호 사이트의 세션 스냅샷은 파티션 정보가 실질적 요구사항이 된다. ([Cloudflare Cookies 문서](https://developers.cloudflare.com/fundamentals/reference/policies-compliances/cloudflare-cookies/))
- 서드파티 쿠키 단계적 폐지와 함께 localStorage/IndexedDB도 최상위 사이트 단위로 **스토리지 파티셔닝**되는 방향이라, "오리진 문자열"만으로는 부족하고 "오리진 × 최상위 사이트" 키가 미래에 안전하다.
- **IndexedDB**는 구조화-복제 가능한 값 전체를 담아 직렬화가 번거롭다. Playwright 최신이 다루기 시작했으나 범용 구현은 드물다. 인증 토큰을 IndexedDB에 두는 사이트는 프로파일 재사용(§1.2)이 현실적 유일책.

**OxiBrowser 현황**: `network/cookie.rs`의 `CookieEntry`가 `sameSite`(Playwright 철자 호환), `httpOnly`, `secure`, `expiry`(세션 쿠키=`None`), `partitioned: bool`, `partition_key: Option<String>`까지 이미 직렬화한다. 갭: (a) `partition_key`가 문자열 하나로 단순화되어 `hasCrossSiteAncestor` 비트가 없고, (b) Phase 8 이전까지 실제 최상위 사이트가 스레딩되지 않아 쿠키 자기 도메인으로 디폴트된다는 주석이 있다. (c) localStorage export가 단일 오리진, sessionStorage/IndexedDB 미지원.

---

## 2. 실제 사용자 브라우저 프로파일에 CDP attach하기

### 2.1 패턴

두 가지 접근이 널리 쓰인다.

1. **수동 CDP**: `chrome --remote-debugging-port=9222 --user-data-dir=<프로파일>`로 실행한 뒤 클라이언트가 `http://127.0.0.1:9222`에 WebSocket 연결. 
2. **에이전트 자동연결**: Chrome DevTools MCP의 auto-connect처럼 사용자가 브라우저에서 원격 디버깅을 승인하면 에이전트가 **개인 프로파일의 열린 탭·쿠키·스토리지에 접근**해 "다시 로그인 없이 인증 세션 재사용"이 가능해진다. ([Chrome DevTools MCP 블로그](https://developer.chrome.com/blog/chrome-devtools-mcp-debug-your-browser-session), [auto-connect 문서](https://developer.chrome.com/docs/devtools/agents/use-cases/auto-connect))

### 2.2 Chrome 136의 중요한 변화

인포스틸러가 원격 디버깅으로 인증 쿠키를 훔치는 공격이 늘자, **Chrome 136부터 기본 데이터 디렉터리에서는 원격 디버깅 스위치가 무시되고, 커스텀 `--user-data-dir`과 세트로만 동작한다.** 즉 "내 일상 프로파일에 그냥 attach"는 이제 원천 차단되며, 우회하려면 프로파일을 복제해야 한다. ([Chrome 블로그: Changes to remote debugging switches](https://developer.chrome.com/blog/remote-debugging-port))

### 2.3 위험 평가

attach된 클라이언트는 사실상 프로파일 전체의 권한을 갖는다:

- 열린 탭 읽기·JS 주입·탐색 조작, 쿠키/스토리지 접근, 이미 로그인된 세션으로 임의 행동(설정 변경, API 키 발급, 결제). ([auto-connect 문서](https://developer.chrome.com/docs/devtools/agents/use-cases/auto-connect))
- **프롬프트 인젝션**: 악성 페이지 콘텐츠가 에이전트에게 지시를 주입해 인증된 세션으로 유출·행동을 유도할 수 있다. Chrome은 오리진 제한·최소 데이터 수집·상태 변경 시 인간 확인을 권고. ([Chrome Agent Security](https://developer.chrome.com/docs/agents/security))
- **CDP 엔드포인트 = 특권 제어 인터페이스.** LAN/VPN/Tailscale/Docker 브리지로 노출하면 안 된다. `Storage` 도메인에 쿠키 조회·조작 오퍼레이션이 있다. ([CDP Storage](https://chromedevtools.github.io/devtools-protocol/tot/Storage/))
- 프로파일 복제물도 "복제 순간의 활성 세션을 담은 자격증명 저장소"라 자동화 전용 저권한 프로파일이지 실제 개인 프로파일의 사본이어서는 안 된다. ([Chrome 블로그](https://developer.chrome.com/blog/remote-debugging-port))

실무 위험 등급(요약): 실제 일상 프로파일 + 신뢰할 수 없는 에이전트 = 최상위 위험; 전용 자동화 프로파일 + 루프백 CDP + 승인된 에이전트 = 통제 가능.

**OxiBrowser 관점**: `oxibrowser-cdp`는 남의 Chrome에 붙는 클라이언트가 아니라 **자체 브라우저를 CDP 서버로 노출**하는 구조(`server.rs`). 따라서 §2의 "실제 프로파일 attach" 리스크는 해당하지 않지만, CDP 서버를 열어두는 순간 같은 특권 인터페이스 문제가 뒤집힌다 — 세션 저장소가 있는 OxiBrowser 인스턴스의 CDP 포트는 루프백 바인딩·소켓 권한 제한이 필수다.

---

## 3. 봇 탐지 — Cloudflare Turnstile/WAF와 자동화 로그인

### 3.1 탐지 구조

Cloudflare는 단일 "headless 플래그"가 아니라 레이어를 쌓는다. ([Bot Detection Engines](https://developers.cloudflare.com/bots/concepts/bot-detection-engines/), [Bot Score](https://developers.cloudflare.com/bots/concepts/bot-score/))

1. **히스토릭/휴리스틱**: UA 누락·빈 UA(→ bot score 1), 헤더 순서가 브라우저 주장과 불일치, IP/ASN 평판.
2. **JavaScript Detections**: 응답 HTML에 주입된 탐침이 **헤드리스 브라우저·자동화 도구 지문을 식별**하도록 설계된 엔진. 브라우저 API 가용성/동작, UA와 노출 기능의 불일치, Canvas/WebGL 특성, 타이밍을 본다.
3. **ML Bot Score (1–99)**: 1–29 통상 자동, 30–99 통상 인간. `__cf_bm` 쿠키(30분)가 세션 스무딩에 사용됨.
4. **Managed Challenge / Turnstile**: 적응형 클라이언트 검증(Proof-of-Work 포함)을 통과하면 clearance 발급. ([How Challenges Work](https://developers.cloudflare.com/cloudflare-challenges/concepts/how-challenges-work/))

핵심: **클라이언트 연속성**도 검증 요소다. 챌린지를 받은 IP와 다른 IP에서 풀면 실패하고 챌린지 루프에 빠진다(공식 문서 명시). 같은 문서가 프록시 체인·쿠키 스트리핑·UA 재작성·이그레스 IP 변경을 점검하라고 지시한다.

### 3.2 `cf_clearance`의 성질 — 세션 재사용의 실전 제약

- 쿠키는 **"특정 방문자·디바이스에 안전하게 결속"**되어 기계 간 재사용을 막는다고 공식 문서가 명시. IP+UA만 맞춰도 재생은 보장되지 않고, Precursor 시스템이 clearance를 지속 재평가해 의심 행동 시 만료 전 무효화될 수 있다. ([Clearance 개념](https://developers.cloudflare.com/cloudflare-challenges/concepts/clearance/))
- 기본 유효기간은 Challenge Passage 설정(기본 30분). ([Challenge Passage](https://developers.cloudflare.com/cloudflare-challenges/challenge-types/challenge-pages/challenge-passage/))
- 속성이 `SameSite=None; Secure; Partitioned`이므로 스냅샷에 파티션 키 보존이 필요(§1.3). ([Cloudflare Cookies](https://developers.cloudflare.com/fundamentals/reference/policies-compliances/cloudflare-cookies/))
- Turnstile 프리클리어스 쿠키는 위젯에 등록된 호스트네임/존에만 유효 — 범용 토큰이 아니다. ([Pre-clearance](https://developers.cloudflare.com/turnstile/additional-configuration/hostname-management/pre-clearance/))

**실무 함의**: `cf_clearance`를 "저장해두고 나중에 재생"하는 설계는 신뢰할 수 없다. 재사용이 그나마 작동하는 조건은 **같은 egress IP + 같은 User-Agent(정확히는 일관된 클라이언트 지문) + 유효기간 내**이며, 그마저 보장이 아니다. 챌린지가 필요한 사이트는 §3.3의 완화와 상승 전환을 조합해야 한다.

### 3.3 완화 전략 (정당한 자동화 전제)

**① 지문 일관성 (spoofing이 아니라 정합성)**: 개별 신호를 위조하기보다 전체가 하나의 그럴듯한 기기를 묘사하게 유지한다. 검증 항목 — `navigator.userAgent` ↔ UA Client Hints ↔ HTTP 헤더 일치, `navigator.platform`/폰트/키보드 ↔ OS 주장, `language(s)` ↔ `Accept-Language` ↔ 타임존, `screen.*`/viewport/DPR 정합성, WebGL vendor/renderer가 주장 OS와 모순되지 않기, `navigator.webdriver`(WebDriver 표준 신호, [W3C spec](https://www.w3.org/TR/webdriver/)), 쿠키/스토리지 연속성, 입력·타이밍 리듬. **"모든 신호를 평범하게"가 아니라 조합의 일관성**이 핵심 — Mac UA + Windows식 WebGL 렌더러 같은 조합이 개별 희소성보다 수상하다. 2025–26 감시 시스템은 CDP/자동화 프로토콜의 부작용까지 상관 분석한다.

**② 네트워크 연속성**: 챌린지 수령·해결·후속 요청이 같은 egress IP에서 나가야 한다. 프록시 로테이션은 세션 스토어와 근본적으로 상충한다 — 세션 재사용 에이전트는 **고정 egress**가 원칙.

**③ 인간 상승 전환 (human escalation)**: Managed/Passive 챌린지는 자동 통과를 시도하고, **Interactive(체크박스/캡차)나 Blocked로 분류되면 즉시 자동 시도를 멈추고 사용자에게 핸드오프**한다. 이는 정합성 있는 전략이자(재시도는 리스크 점수만 악화) 실제 사례의 "사용자 손 빌림"을 최소 단계로 좁히는 방법이다. 1회 인간 승인으로 얻은 세션을 저장해 이후를 무인화하는 구성(§4.5)이 최적 균형점.

**④ OxiBrowser 현황**: `challenge.rs`가 Cloudflare/DataDome/PerimeterX를 `ChallengeVendor`로, 종류를 Managed/Interactive/Blocked 등 `ChallengeKind`로 분류하고 클리어런스 쿠키명(`cf_clearance`/`datadome`/`_px3`)까지 인지한다. `network/client.rs`의 챌린지 재시도 루프는 Interactive/Blocked에서 중단하고 결과를 반환한다 — 즉 **상승 전환의 감지 계층이 이미 존재**하며, 남은 것은 이 감지를 에이전트 워크플로의 "인간 핸드오프" 이벤트로 연결하는 것. `js/stealth.rs`와 pure-Rust stealth 설계 문서(`designs/2026-06-25-pure-rust-stealth.md`)가 지문 일관성 작업을 담당한다.

### 3.4 사례 매핑

- **Cloudflare 터널 인증**: `cloudflared tunnel login`은 브라우저에서 콘솔 승인을 받는 OAuth류 흐름. 콘솔 로그인 세션(cloudflare.com 쿠키)이 저장소에 있으면 재승인 무인화 가능. 단 Cloudflare 자체 사이트라 챌린지·리스크 평가가 활성 — 지문 일관성 + 고정 egress가 전제.
- **Tailscale 관리콘솔 승인**: login.tailscale.com 세션 재사용으로 무인화 가능. SSO 리다이렉트가 끼면 IdP 세션까지 스코프에 포함해야 함.
- **macOS iCloud 로그아웃·시스템설정 GUI**: 브라우저 밖 영역 — 이 문서 범위 밖이나, 설계 원칙(자격증명 저장소·상승 전환)은 그대로 적용됨(별도 문서 03/04 참조).

---

## 4. OxiBrowser용 세션 저장소 설계안

### 4.1 목표와 비목표

- 목표: 1회 인간 승인으로 확보한 인증 세션을 **도메인 스코프 단위로 저장·주입·갱신**하여 에이전트 무인 진행. Cloudflare/Tailscale 콘솔 사례가 1차 고객.
- 비목표: `cf_clearance` 재생으로 챌린지 우회 지속화(§3.2), 사용자 일상 Chrome 프로파일 통째 재사용(§2), macOS 키체인 비밀번호 직접 대신 입력.

### 4.2 포맷 — Playwright 호환 확장 봉투(envelope)

기존 `StorageState`(cookies + origins)를 그대로 재사용하되, 저장소 파일은 다음 봉투로 감싼다:

```jsonc
{
  "version": 1,                       // 포맷 마이그레이션용
  "created_at": "...", "updated_at": "...",
  "scope": "tailscale.com",           // registrable domain 스코프 키
  "fingerprint": {                    // 세션 획득 당시 클라이언트 지문 —
    "user_agent": "...",              // 주입 시 이 값과 다르면 경고/거부
    "client_hints": { "platform": "macOS", ... },
    "locale": "ko-KR", "timezone": "Asia/Seoul"
  },
  "egress": { "kind": "fixed", "note": "home-wan" },  // IP 연속성 힌트
  "state": { "cookies": [...], "origins": [...] }     // 기존 StorageState 그대로
}
```

설계 근거:

- **Playwright 상호교환 유지**: `state` 필드가 기존 `StorageState` JSON 그대로라 덤프가 diff 가능하고 Playwright와 양방향 이동. 이미 `storage_state.rs` 문서가 이를 목표로 명시.
- **쿠키 속성 완전 보존**: `CookieEntry`는 sameSite/httpOnly/expiry/partitioned/partition_key를 이미 직렬화. 갭 보강 — `partition_key`를 `{topLevelSite, hasCrossSiteAncestor}` 구조로 확장(Chrome 128+ 비트, [chromestatus](https://chromestatus.com/feature/5150980794632192)), 세션 쿠키는 `expiry: null`로 저장(§1.3). OxiBrowser는 자체 jar를 직렬화하므로 HttpOnly 제약이 없다는 게 CDP 기반 도구 대비 구조적 이점.
- **지문·egress 메타데이터가 세션과 동반**: §3의 "IP+UA 불일치 → 무효"를 복원 시점에 사전 점검 가능하게 한다. 주입 직전 UA·헤더가 `fingerprint`와 다르면 세션 낭비·리스크 평가 악화 전에 중단.
- **localStorage 스코핑**: 현 구조의 export는 현재 오리진 하나만 담긴다. 저장소는 `origins[]`에 스코프 내 오리진들을 여러 개 담을 수 있어야 하고(예: `login.tailscale.com` + `tailscale.com` + SSO 오리진), import 시 `pending_seed`의 "다음에 로드되는 오리진에 몰아주기" 제약(`js/runtime.rs:2245-2248` 주석)을 오리진 매칭으로 풀어야 한다. sessionStorage는 표준 포맷에 없으므로 v1에서 제외, IndexedDB는 프로파일 재사용(§1.2)이 필요할 때 별도 경로로.

### 4.3 암호화 — at-rest 보호

저장소 파일은 bearer credential이므로 평문 금지:

1. **키 보관**: Rust [`keyring`](https://docs.rs/keyring/latest/keyring/) 크레이트(4.x)로 macOS Keychain / Linux Secret Service / Windows Credential Manager에 32바이트 랜덤 키를 보관. `Entry::get_secret()`→`NoEntry`면 `rand`으로 생성 후 `set_secret()`으로 저장(생성-한-번 패턴). macOS에서 서명 아이덴티티 변경 시 키체인 프롬프트가 날 수 있음은 문서화 필요. ([keyring docs](https://docs.rs/keyring/latest/keyring/))
2. **본체 암호화**: XChaCha20-Poly1305 또는 AES-256-GCM(AEAD)으로 봉투 전체 암호화, nonce는 ciphertext 앞에 붙여 저장. 인증되지 않은 모드(CBC 등) 금지.
3. **폴백 정책**: 키체인 접근 불가 시(헤드리스 리눅스에 Secret Service 데몬 부재 등) **평문 폴백 금지** — 명시적 오류로 중단. (`keyring` 생태계 문서도 샘플 스토어의 비보안성을 명시)
4. 키/평문 로깅 금지, 사용 후 `zeroize` 권장.

대안: 파일 키체인(Secret Service)이 아닌 **macOS Keychain에 세션 자체를 항목으로 넣는** 안도 가능하나, 4KB 항목 한계·검색성·크로스플랫폼 균일성 때문에 "키체인=키, 파일=암호문" 조합이 낫다.

### 4.4 도메인 스코핑과 접근 통제

- **파일 단위 = 스코프 단위**: `~/.oxibrowser/sessions/<registrable-domain>.session`(암호문). 로드는 스코프 키 매칭으로만. 에이전트가 A 사이트 작업 중 B 사이트 쿠키를 읽는 경로를 구조적으로 차단 — §2.3의 프롬프트 인젝션 완화(최소 권한)와 직결.
- **수명·로테이션**: 각 쿠키 `expiry` 기반 만료 관리 + 세션 단위 `updated_at`. 미인증 감지(로그인 폼 재출현 등) 시 해당 스코프만 폐기·재상승. 챌린지 쿠키(`cf_clearance`)는 장수 저장소가 아니라 세션 캐시 취급(30분 통로).
- **감사**: 스코프별 저장/주입/폐기 이벤트 로그(값 제외, 이름·도메인·시각만).
- **동시성**: user-data-dir과 달리 스냅샷 파일은 복사-온-라이트로 읽고 저장 시점에 원자적 치환(tmp+rename) — 여러 세션이 같은 스코프를 동시 쓰는 문제를 회피.

### 4.5 무인 진행 흐름 (전체 그림)

```
[최초 1회]  인간 상승: 챌린지 Interactive/Blocked 또는 미인증 감지
            → 사용자가 전용 프로파일/제어 UI에서 로그인·승인 (1회)
            → export: 쿠키(파티션 키 포함)+localStorage+지문+egress
            → AEAD 암호화 후 스코프 파일로 저장
[이후 무인] 작업 시작 → 스코프 파일 복호화 → 지문/egress 사전 점검
            → import_state로 주입 → 무인 진행
            → 만료/챌린지/미인증 감지 시 해당 스코프만 폐기하고 상승 재요청
```

이 흐름이 실제 사례의 4개 수동 단계 중 브라우저 2개(Cloudflare 터널 인증, Tailscale 콘솔 승인)를 무인화하고, 남은 2개(macOS iCloud, 시스템설정 GUI)는 상승 전환 프리미티브를 공유한다.

---

## 5. 실행 가능한 권고

OxiBrowser에 즉시 적용 가능한 순서:

1. **세션 저장소 뼈대 (신규 `session-store` 모듈)**: §4.2 봉투 포맷(`version`/`scope`/`fingerprint`/`egress`/`state`) + 스코프별 파일 저장·원자적 치환·감사 로그. `state` 필드는 기존 `StorageState`를 그대로 재사용해 Playwright 상호교환 유지.
2. **쿠키 완전성 보강**: `CookieEntry.partition_key`를 문자열에서 `{topLevelSite, hasCrossSiteAncestor}` 구조로 확장하고 import/export에서 왕복(round-trip) 보장. 세션 쿠키는 `expiry: null`로 저장·복원(§1.3 표). `__Secure-`/`__Host-` 접두사 복원 규칙을 `import_state` 검증에 추가.
3. **암호화 계층**: `keyring`(4.x)로 키 보관 + `chacha20poly1305` AEAD로 파일 암호화. 키체인 불가 시 평문 폴백 금지·명시적 에러. Cargo 의존 추가만으로 구현 가능([keyring docs](https://docs.rs/keyring/latest/keyring/)).
4. **다중 오리진 export**: `Session::export_state`의 단일 오리진 한계를 스코프(등록 가능 도메인) 단위 수집으로 확장하고, import 씨드를 "다음 로드 오리진에 몰아주기"에서 오리진 매칭 주입으로 전환(`js/runtime.rs` SetPageUrl 씨드 경로 수정).
5. **상승 전환 연결**: `challenge.rs`가 Interactive/Blocked로 분류하면(`network/client.rs` 중단 조건 이미 존재) 재시도 없이 (a) 해당 스코프 세션 폐기, (b) 에이전트 CLI/MCP에 "인간 승인 필요" 이벤트 노출, (c) 승인 완료 후 자동 export→저장. 스킬 시스템(`skills/`)의 각 스킬에 "사전 상승 필요" 선언 옵션 추가.
6. **지문 일관성 계약**: 세션 저장 시 UA/Client Hints/로케일/타임존을 `fingerprint`로 저장하고, 주입 전 현재 설정과 불일치 시 경고·중단. `js/stealth.rs`의 지품 값을 세션 지문에서 파생시켜 "세션마다 동일한 가상 기기"를 유지(= Playwright 컨텍스트 재사용과 동등한 일관성).
7. **egress 고정 원칙**: 세션 스코프가 활성인 동안 프록시/네트워크 인터페이스 변경을 금지하는 설정(또는 경고). Cloudflare 챌린지 루프의 공식 원인이 IP 불연속(§3.1).
8. **CDP 서버 보호**: 세션 저장소를 가진 인스턴스의 `oxibrowser-cdp` 서버는 루프백 전용 바인딩·소켓 퍼미션 0600을 기본값으로(§2.3).

**명시적 비권고**: `cf_clearance` 스냅샷 재생에 의존한 챌린지 우회, 사용자 일상 Chrome 프로파일 attach(Chrome 136 이후 차단 + 최상위 위험), IndexedDB 인증 사이트를 위한 범용 직렬화(프로파일 재사용이 정답).

---

## 주요 출처

1. Playwright — Authentication: https://playwright.dev/docs/auth
2. Playwright — Test generator (codegen `--save-storage`): https://playwright.dev/docs/codegen
3. Chromium — User Data Directory (프로파일 잠금·단일 인스턴스·보안): https://chromium.googlesource.com/chromium/src/%2B/HEAD/docs/user_data_dir.md
4. CDP — Target (browser contexts 격리): https://chromedevtools.github.io/devtools-protocol/tot/Target/
5. CDP — Storage (`setCookies`/`partitionKey`/`__Secure-` 복원 제약): https://chromedevtools.github.io/devtools-protocol/tot/Storage/
6. Chrome 블로그 — Changes to remote debugging switches (Chrome 136 기본 프로파일 디버깅 차단): https://developer.chrome.com/blog/remote-debugging-port
7. Chrome — DevTools for agents: auto-connect (개인 프로파일 재사용·위험): https://developer.chrome.com/docs/devtools/agents/use-cases/auto-connect
8. Chrome — Agent security (프롬프트 인젝션·오리진 제한·인간 확인): https://developer.chrome.com/docs/agents/security
9. Cloudflare — Bot detection engines (JS Detections가 헤드리스 식별): https://developers.cloudflare.com/bots/concepts/bot-detection-engines/
10. Cloudflare — Bot scores (UA 누락→score 1, 1–29/30–99): https://developers.cloudflare.com/bots/concepts/bot-score/
11. Cloudflare — How Challenges work (챌린지 중 IP 변경 → 루프): https://developers.cloudflare.com/cloudflare-challenges/concepts/how-challenges-work/
12. Cloudflare — Clearance (`cf_clearance` 방문자·디바이스 결속, 재평가): https://developers.cloudflare.com/cloudflare-challenges/concepts/clearance/
13. Cloudflare — Challenge Passage (기본 30분): https://developers.cloudflare.com/cloudflare-challenges/challenge-types/challenge-pages/challenge-passage/
14. Cloudflare — Cookies 정책 (`cf_clearance` 속성 `SameSite=None; Secure; Partitioned`, `__cf_bm`): https://developers.cloudflare.com/fundamentals/reference/policies-compliances/cloudflare-cookies/
15. Privacy Sandbox — CHIPS (쿠키 파티셔닝): https://privacysandbox.google.com/cookies/chips
16. Rust `keyring` 크레이트 (Keychain/Secret Service/Credential Manager 키 보관): https://docs.rs/keyring/latest/keyring/

보조: W3C WebDriver(`navigator.webdriver`) https://www.w3.org/TR/webdriver/ · MDN CHIPS https://developer.mozilla.org/en-US/docs/Web/Privacy/Partitioned_cookies · Chrome 128 파티션 키 비트 https://chromestatus.com/feature/5150980794632192 · Cloudflare Turnstile pre-clearance https://developers.cloudflare.com/turnstile/additional-configuration/hostname-management/pre-clearance/ · Chrome DevTools MCP 블로그 https://developer.chrome.com/blog/chrome-devtools-mcp-debug-your-browser-session
