# 05. AI 에이전트 브라우저 제품의 인증·자격증명 처리 방식 비교 조사

- 작성일: 2026-09-27
- 목적: OxiBrowser(pure-Rust 헤드리스, CDP, session/executor, HAR 캡처, 스킬 시스템)가 사용자 자격증명(키체인 등)을 안전하게 재사용해 무인 진행하기 위한 선행 사례 조사
- 방법: 공식 문서·시스템 카드·GitHub 소스 기반 웹 조사 (READ-ONLY)

---

## 1. 요약 비교표

| 제품 | 로그인 처리 | 격리 모델 | 확인/동의 UX | 세션·쿠키 저장 |
|---|---|---|---|---|
| **OpenAI Operator → ChatGPT Agent** | 원격 가상 브라우저에서 "브라우저 인계(takeover)"로 사용자가 직접 입력. 비밀번호 채팅 입력 금지. cloud browser에는 secure form(프롬프트 거쳐가지 않고 원격 브라우저로 직접 전달) | 자체 관리 원격 브라우저(컨테이너) + 사이트 블록리스트. 엔터프라이즈는 도메인 단위 차단·허용목록. CUA API는 VM/컨테이너 + 이그레스 도메인 제한 권고 | 고영향 액션(구매·전송·게시) 사용자 확인, 민감 사이트 **watch mode**(감시자 이탈 시 자동 일시정지), 프롬프트 인젝션 모니터링 | 쿠키 세션 간 지속("일반 브라우저처럼"). 데이터 컨트롤에서 로그아웃/쿠키 삭제. 인계 중 스크린샷 미캡처 |
| **Anthropic computer use / Claude for Chrome** | API computer use는 로그인 포함 전부 개발자 구현(인간이 로그인 수행 or 단기 자격증명 권고). Claude for Chrome은 **사용자의 실제 Chrome 프로파일 인계**(이미 로그인된 세션 재사용). 커넥터는 OAuth 위임 | computer use: 실행 환경(VM/컨테이너)은 개발자 소유. Claude for Chrome: 컨테이너 격리 없음 — Chrome 확장 권한 모델 + 사이트 차단 + 제품측 분류기가 경계 | Claude for Chrome: 게시·구매 등 고위험 액션 확인 프롬프트, 사이트 블록리스트. API에는 기본 확인 UX 없음(개발자 몫) | 확장은 사용자 Chrome 프로파일의 쿠키·세션을 그대로 사용(별도 저장 정책 없음). API는 구현자 몫 |
| **Browser Use (OSS)** | 3안: ① `storage_state` JSON(쿠키+localStorage) 저장/재사용 ② 실제 Chrome 프로파일 인계(`from_system_chrome`, `user_data_dir`) ③ `sensitive_data` 플레이스홀더(`<secret>name</secret>`, 실행 직전 치환, 도메인별 스코프) | 로컬 Playwright 기반 Chromium(VM 아님). `allowed_domains` 도메인 허용목록 옵션. sensitive_data 사용 시 allowed_domains 권장 | 프레임워크 차원 사전 승인 훅 없음 — 커스텀 `ask_human` 액션 패턴(LLM 지시 기반이라 우회 가능, 반면교사) | `export_storage_state()` / `save_cookies()` / storage_state watchdog. 권고: 파일 chmod 600 + gitignore |
| **Steel** | **Profiles**(user-data-dir 스냅샷, persistProfile) + **Credentials API**(origin+namespace로 저장, 세션에 주입 — 프롬프트에 원문 미노출) | 세션별 격리 클라우드 브라우저. namespace로 프로파일·자격증명 스코프 분리. 프로파일 300MB·30일 비활용 자동 삭제 | `autoSubmit: false`(채우기는 하나 제출 전 승인 경계), `blurFields`(입력값 블러), `exactOrigin`(지정 오리진에만 주입) | 세션 release 시 스냅샷 저장. `persistProfile:false`로 읽기 전용 재사용. 동일 프로파일 동시 쓰기 금지 권고 |
| **Browserbase** | **Contexts**(persist:true) + **Session Live View**로 1회 수동 로그인 → 이후 세션 자동 로그인. **1Password 볼트 통합**(agentic autofill). 쿠키 직접 저장/복원 API도 제공 | 세션마다 fresh user data directory(기본 리셋)되는 클라우드 컨테이너 + 지오 프록시 + "Verified" 핑거프린트. **Web Bot Auth**(에이전트 암호적 신원) | Live View 원격 제어로 2FA·CAPTCHA 인간 개입. RBAC로 "큰 거래만 승인, 잦은 승인 프롬프트는 지양" 철학 명시 | Context는 쿠키·localStorage·IndexedDB·SW·자동입력 등 UDD 전체를 **암호화 저장**, 무기한 존속(명시 삭제까지). 동시 로그인 회피, 사이트·로그인당 1컨텍스트 권고 |
| **Stagehand** | LOCAL: Playwright `userDataDir` 영구 프로파일(1회 수동 로그인). BROWSERBASE: 컨텍스트 persist. `act()`의 `variables`로 민감값 LLM 미전달 후 치환. `context.addCookies()` 수동 주입도 가능 | 자체 실행 환경(개발자 로컬 또는 Browserbase 클라우드). 도메인 허용목록 등 별도 격리 기능은 문서상 없음 | act/extract/observe/agent 프리미티브 수준 — 확인 게이트는 앱 영역 (프레임워크 미제공) | 로컬 프로파일 디렉터리 또는 Browserbase Context로 지속. 동일 프로파일 동시 실행 금지 |
| **Skyvern** | ① 저장된 password credential + **TOTP 시크릿**(LLM 미전달) ② **Bitwarden/vaultwarden 통합**(엔터프라이즈, `bw serve` CLI 브리지) ③ 커스텀 credential service ④ 브라우저 세션/프로파일 재사용 | 클라우드 격리 세션 or **로컬 Chrome CDP 연결**(`--remote-debugging-port=9222`, `skyvern browser serve --tunnel`). 프록시 위치 지정 | 워크플로 블록 기반, 자격증명·민감 필드 로그 마스킹. 확인 게이트는 워크플로 구성 몫 | `browser_session_id`(라이브 세션), `browser_profile_id`(프로파일 지속). 동시 실행 시 Chromium 프로파일 잠금 충돌 이슈 존재 |

---

## 2. 제품별 상세

### 2.1 OpenAI Operator → ChatGPT Agent (현재 ChatGPT Work / cloud browser로 통합)

**로그인 처리 — "인계(takeover)" 모델의 원형.** Operator는 원격 가상 브라우저에서 동작하며, 로그인이 필요하면 작업을 멈추고 사용자에게 브라우저 제어권을 넘긴다("…" → Take over browser). 사용자가 직접 비밀번호·MFA·패스키를 입력하고 제어권을 반납하면 에이전트가 이전 상태에서 재개한다. **인계 중에는 스크린샷을 캡처하지 않아** 입력된 비밀번호가 관찰 파이프라인(모델 컨텍스트·기록)에 들어가지 않는다. 비밀번호를 채팅에 입력하는 것은 금지 안내. 후속 cloud browser에서는 자격증명을 채팅 프롬프트가 아닌 **secure form을 통해 원격 브라우저로 직접 전달**하는 흐름이 추가됐다. ([help.openai.com — ChatGPT agent](https://help.openai.com/en/articles/11752874-chatgpt-agent), [help.openai.com — cloud browser](https://help.openai.com/en/articles/20001280-using-cloud-browser-in-chatgpt), [Operator System Card](https://openai.com/index/operator-system-card/))

**격리 모델.** 자체 관리하는 원격 브라우저(컨테이너)에서 실행되며, 보안·안전·컴플라이언스 이유로 일부 사이트에 접근 불가(블록리스트, 가상 브라우저+커넥터 공통 적용). 엔터프라이즈는 워크스페이스 단위 사이트/도메인 차단(서브도메인 전체 `.example.com` 포함)과 allowlisting 지원. API(Computer-Using Agent) 개발자 가이드는 전용 VM/컨테이너, 승인된 도메인으로 아웃바운드 제한, VM 내 마스터 키·무관 자격증명 배치 금지를 권고한다. ([Sandbox security](https://developers.openai.com/api/docs/guides/agents-api/environments/security))

**확인/동의 UX.** ① 고영향 액션(구매, 메시지 전송, 계정 변경 등)에 사용자 확인 ② 민감 사이트(이메일 등)에서 **watch mode** — 사용자가 페이지를 떠나거나 비활성화되면 자동 일시정지, 복귀 시 재개 ③ 프롬프트 인젝션 모니터링 ④ 금지 작업 거부 패턴. 시스템 카드는 이들이 "완화책이지 제거책이 아님"을 명시한다. ([Operator System Card](https://openai.com/index/operator-system-card/))

**세션·쿠키 정책.** "쿠키는 일반 브라우저처럼 세션 간 지속"되며, 저장된 접근 제거는 사이트 로그아웃 + ChatGPT 데이터 컨트롤에서 쿠키 삭제로 수행. 대화·브라우징 기록·스크린샷은 대화 삭제 시까지 보존(삭제 후 90일 내 시스템 제거). 1Password 측은 "잠금 해제된 1Password 확장이 AI 브라우징에 노출되는 프롬프트 인젝션 위험"에 대한 보안 어드바이저리를 발행 — 패스워드 매니저·에이전트 브라우저 조합의 위험을 보여주는 사례. ([1Password 보안 어드바이저리](https://1password.com/blog/security-advisory-for-ai-assisted-browsing-with-the-1password-browser))

### 2.2 Anthropic computer use / Claude for Chrome

**두 제품, 두 철학.** API computer use는 스크린샷→마우스/키보드 액션 프로토콜만 제공하고 **실행 환경 전체(브라우저, VM, 로그인)는 개발자 소유**다. 로그인은 인간이 상호작용으로 완료하거나 단기·최소권한 자격증명을 쓰도록 권고. 반면 Claude for Chrome(확장)은 **사용자의 실제 Chrome 프로파일에서 동작**하는 프로파일 인계형이다 — 이미 로그인된 세션을 그대로 물려받으므로 비밀번호 평문 노출은 없지만, "로그인된 세션 쿠키 자체가 자격증명과 동등한 권한"이라는 트레이드오프가 있다. ([Anthropic computer use 발표](https://www.anthropic.com/news/3-5-models-and-computer-use), [Claude for Chrome 지원 문서](https://support.anthropic.com/zh-CN/articles/12012173-chrome%E7%89%88claude%E5%85%A5%E9%97%A8%E6%8C%87%E5%8D%97))

**격리 모델.** Claude for Chrome은 컨테이너 격리가 아니라 ① Chrome 확장 권한 모델 ② 세밀한 권한·사이트 블록리스트 ③ 고위험 액션 세이프가드의 조합이 경계다. 서버측 코드 실행(gVisor 격리)과는 다름을 명확히 구분해야 한다. ([How we contain Claude](https://www.anthropic.com/engineering/how-we-contain-claude))

**확인/동의 UX.** 게시·구매 등 일부 액션에 확인 프롬프트. 주요 위협은 간접 프롬프트 인젝션(페이지 내용에 속아 로그인된 권한으로 행동)이며, Anthropic은 탐지 분류기·시스템 프롬프트 개선으로 공격 성공률을 낮췄지만 0이 아니라고 보고. ([Prompt injection defenses](https://www.anthropic.com/research/prompt-injection-defenses))

**세션·쿠키 정책.** 확장은 사용자 Chrome 프로파일 정책을 그대로 따름(별도 저장 정책 없음). 권장 운용: 에이전트 전용 Chrome 프로파일을 만들고 작업에 필요한 계정만 로그인. OAuth 커넥터는 비밀번호 대신 스코프 기반 위임(철회 가능)을 사용 — "비밀번호 입력보다 안전하지만 토큰 스코프는 여전히 민감". ([원격 MCP 커넥터](https://support.anthropic.com/en/articles/11175166-about-custom-integrations-using-remote-mcp))

### 2.3 Browser Use (오픈소스)

**로그인 처리 — 실용적 3안.** ① 로그인 1회 → `export_storage_state("auth.json")`로 쿠키+localStorage 저장 → 이후 `BrowserProfile(storage_state=...)`로 재사용 ② `Browser.from_system_chrome(profile_directory=...)`로 **실제 Chrome 프로파일 인계**(인증 상태 export 가능) ③ 전용 `user_data_dir` 지속 프로파일. ([browser.md 스킬 문서](https://github.com/browser-use/browser-use/blob/main/skills/open-source/references/browser.md), [save_cookies 예제](https://github.com/browser-use/browser-use/blob/main/examples/browser/save_cookies.py))

**자격증명 비노출: `sensitive_data`.** 태스크에는 플레이스홀더(`<secret>user_name</secret>`)만 두고, 실제 값은 모델이 액션을 생성한 **후 실행 직전 파라미터에서 치환**된다. 도메인 키로 스코프(`{"https://app.example.com": {...}}`) 가능하며, 인젝션 악용 방지를 위해 `allowed_domains` 병행을 공식 권장. 단, "LLM 의도적 입력으로부터의 보호"일 뿐 스크린샷·페이지 관찰의 누출까지 보장하지 않는다. ([sensitive_data 예제](https://github.com/browser-use/browser-use/blob/main/examples/features/sensitive_data.py), [구현](https://github.com/browser-use/browser-use/blob/main/browser_use/tools/registry/service.py))

**격리·확인 UX.** 로컬 Playwright Chromium(VM 없음), `allowed_domains` 허용목록. **사전 실행 승인 훅은 프레임워크에 없다** — 커스텀 `ask_human` 액션으로 흉내 내며, 이는 LLM 지시 기반이라 모델이 우회할 수 있다(강제 승인은 액션 실행 계층에서 별도 구현 필요). ([AGENTS.md](https://github.com/browser-use/browser-use/blob/main/AGENTS.md))

**세션·쿠키 정책.** storage_state 파일은 "재사용 가능한 세션 쿠키를 포함"하므로 chmod 600 + gitignore 권고. storage_state와 user_data_dir 동시 사용 시 충돌 경고, 병렬 실행에는 storage_state 복사본 권장. 세션 만료·IP/핑거프린트 바인딩·MFA는 여전히 수동 개입 필요로 명시.

### 2.4 Steel

**Profiles + Credentials의 2층 구조가 가장 정돈된 설계.** **Profiles**는 세션을 `persistProfile: true`로 만들고 release하면 user-data-dir(쿠키·localStorage·확장·자격증명 포함)을 스냅샷으로 저장, 이후 `profileId`로 재사용. `persistProfile: false`면 읽기 전용(병렬 워커에 안전). **Credentials API**(beta)는 자격증명을 **origin + namespace**로 저장하고 세션에 주입 — 에이전트(프롬프트)는 원문 username/password/TOTP를 받지 않는다. 기본값 `autoSubmit`(자동 제출), `blurFields`(입력 필드 블러), `exactOrigin`(지정 오리진에만 주입)는 명시적 설정 권고. ([Profiles 개요](https://docs.steel.dev/overview/profiles-api/overview), [Credentials API](https://llms.steel.dev/articles/credentials-api-for-browser-agents/))

**운용 가이드가 구체적.** "credential으로 인증을 수립/복구하고, 결과 상태를 profile로 지속하라" 플로우, 프로파일당 계정·환경 분리(prod/staging 네임스페이스), `autoSubmit: false`를 승인 경계로 사용, 프로파일 복원 후 **로그인 상태 검증 → 실패 시 재인증 라우팅**(무한 재시도 금지), 네트워크 아이덴티티(IP) 일관성을 위해 전용 IP 페어링 권고. 제약: 프로파일 300MB, 30일 비활용 시 자동 삭제. ([Steel 블로그](https://steel.dev/blog/profiles))

### 2.5 Browserbase

**Contexts = 암호화된 영구 컨텍스트.** 기본적으로 모든 세션은 fresh user data directory로 시작(상태 리셋). Context를 만들고 `persist: true`로 세션을 띄우면 쿠키·localStorage·IndexedDB·sessionStorage·Service Worker·자동입력(Web Data)·사이트 권한/HSTS까지 UDD 전체가 저장되며 **컨텍스트 단위로 암호화 저장**된다(HTTP 캐시는 제외). 표준 로그인 플로우: ① Context 생성 ② Live View(원격 제어 뷰어)로 수동 로그인 ③ 세션 종료 + 수 초 대기(동기화) ④ 이후 동일 contextId 세션은 자동 로그인. ([Contexts 문서](https://docs.browserbase.com/platform/browser/core-features/contexts), [인증 가이드](https://docs.browserbase.com/platform/identity/authentication))

**인증 3층 스택.** ① **Web Bot Auth** — Cloudflare·Stytch 등과 만든 개방 표준 기반 암호적 에이전트 신명(신원 증명) ② **1Password 볼트 접근** — "사용자가 이미 신뢰하는 자격증명 저장소를 중복 없이 재사용"하는 agentic autofill ③ **Contexts** — 1회 로그인 후 세션 재사용. RBAC 철학: "큰 거래는 승인, 잦은 승인 프롬프트는 지양". ([Browserbase Identity](https://www.browserbase.com/identity), [1Password 제휴 발표](https://www.browserbase.com/blog/1password-agentic-autofill))

**검증된 운용 규칙.** 동일 컨텍스트 동시 세션 금지(사이트가 강제 로그아웃 유발), 사이트·로그인당 1컨텍스트, 지오 프록시로 위치 일관성, 2FA는 Live View로 최종 사용자에게 원격 제어 반환 또는 앱 비밀번호 사용. 컨텍스트는 무기한 존속하지만 사이트측 만료(쿠키 만료, 비밀번호 변경, 서버측 로그아웃, 토큰 철회)는 별도이므로 **로그아웃 감지→재인증 체크를 자동화에 넣으라**고 명시.

### 2.6 Stagehand

**인증은 "프리미티브 + 호스팅 환경 위임".** Stagehand 자체는 act/extract/observe/agent 프리미티브를 제공할 뿐 인증 프레임워크가 없다. LOCAL 환경에서는 Playwright `userDataDir` 영구 프로파일로 "1회 수동 로그인 → 쿠키 지속" 패턴(README 공식 예시), BROWSERBASE 환경에서는 Context+persist를 넘긴다. 민감값은 `act()`의 `variables`(%email% 등)로 전달 — **변수 값은 LLM 제공자에게 전송되지 않고** 실행 시 치환된다. MFA/SSO는 "사람이 보이는 브라우저에서 완료" 패턴 권장. ([Stagehand README](https://github.com/browserbase/stagehand), [act 문서](https://github.com/browserbase/stagehand/blob/main/packages/docs/v3/references/act.mdx))

**쿠키 API.** `context.cookies()/addCookies()/clearCookies()`로 명시적 주입도 가능. 단 `document.cookie` 복사는 HttpOnly 누락 위험으로 "프로파일/Context 지속을 우선하라"고 가이드. 확인 게이트·도메인 허용목록은 앱 영역.

### 2.7 Skyvern

**자격증명 저장소 접근이 가장 다양.** ① Credentials 페이지/API에 username/password + **TOTP 시크릿** 저장 — "LLM에 보내지 않고" 로그인에 사용 ② **Bitwarden 통합**(엔터프라이즈): 전용 컬렉션을 공유하고 에이전트가 실시간 조회. 자체 호스팅은 vaultwarden + `bw serve`(CLI REST 브리지) 구성, 마스터 비밀번호·API 키는 env로 주입 ③ 커스텀 credential service 연동 ④ 워크플로 파라미터 + 민감 데이터 마스킹(로그·LLM 페이로드에서 [MASKED] 처리). ([passwords.mdx](https://github.com/Skyvern-AI/skyvern/blob/main/fern/credentials/passwords.mdx), [bitwarden.mdx](https://github.com/Skyvern-AI/skyvern/blob/main/fern/credentials/bitwarden.mdx))

**세션 모델.** `browser_session_id`로 라이브 브라우저를 여러 태스크에 걸쳐 재사용(기본 타임아웃 60분, 5~120분), `browser_profile_id`/persistent 세션으로 런 간 지속. **로컬 Chrome CDP 연결**(`--remote-debugging-port=9222`)과 `skyvern browser serve --tunnel`(클라우드가 내 브라우저 제어, API 키 보호 필수)도 지원. 검증된 실패 사례가 교훈적: persist 플래그가 세션을 저장만 하고 재실행 시 임시 user_data_dir을 쓰는 버그, 동시 실행 시 Chromium 프로파일 잠금 충돌. ([skill.md](https://github.com/Skyvern-AI/skyvern/blob/main/docs/skill.md), [이슈 #3897](https://github.com/Skyvern-AI/skyvern/issues/3897), [이슈 #4390](https://github.com/Skyvern-AI/skyvern/issues/4390))

---

## 3. 횡단 분석 — 수렴하는 패턴

1. **비밀번호는 결코 프롬프트/모델 컨텍스트로 보내지 않는다.** OpenAI(secure form/인계), Steel(Credentials API 주입), browser-use(sensitive_data 치환), Stagehand(variables), Skyvern(저장소 조회) 모두 동일한 원칙. 차이는 "누가 채우는가"뿐: 사용자(인계), 서비스(주입), 로컬 실행 직전 치환.
2. **인증 상태는 2층으로 나뉜다.** ① 브라우저 상태(쿠키·스토리지)의 영속 — 프로파일/컨텍스트 ② 원천 자격증명(비밀번호·TOTP)의 보관 — 자격증명 저장소/볼트. Steel의 "credential으로 수립 → profile로 지속 → 만료 시 credential으로 복구" 플로우가 정석.
3. **격리와 편의는 트레이드오프다.** 원격 컨테이너(OpenAI, Steel, Browserbase)는 안전하지만 사용자 환경과 단절 → 로그인 인계 UX가 필수. 확장/CDP 인계형(Claude for Chrome, Skyvern local, browser-use from_system_chrome)은 편하지만 "세션 쿠키 = 자격증명" 위험을 통째로 안는다.
4. **확인 게이트의 강도 스펙트럼.** 프롬프트 기반 요청(browser-use ask_human, 우회 가능) < 제출만 막는 구성 게이트(Steel autoSubmit:false) < 실행 계층의 분류기+확인(OpenAI 고영향 확인·watch mode) < 도메인 차단(OpenAI/Anthropic 블록리스트, browser-use allowed_domains).
5. **영속 상태의 운용 규칙이 공통.** 동시 실행 잠금(Skyvern 이슈가 반면교사), 읽기 전용 vs 쓰기 모드 분리(Steel), 저장 데이터 암호화(Browserbase), 로그인 상태 재검증 + 만료 시 재인증 라우팅(Steel·Browserbase), 파일 권한 관리(browser-use).
6. **최신 방향: 에이전트 신원 + 기존 볼트 재사용.** Browserbase의 Web Bot Auth(사이트가 에이전트를 검증)와 1Password/Bitwarden 통합(사용자 자격증명 생태계 재사용)이 2025~2026년의 수렴점.

---

## 4. OxiBrowser가 복사할 만한 패턴 Top 5

OxiBrowser는 로컬 실행·pure-Rust·CDP 노출·session/executor 구조를 가지므로, "클라우드 컨테이너" 패턴보다 **로컬 신뢰 경계를 프레임워크화하는 패턴**이 직접 적용 가능하다.

### 1) 브라우저 인계(Takeover) 게이트 — OpenAI Operator / Browserbase Live View
- 로그인·MFA·OAuth 승인 단계에서 CDP 입력 권한을 사용자에게 독점 위임하고, **그 구간 동안 스크린샷·DOM 스냅샷·HAR 캡처를 중단**(Operator의 "인계 중 미캡처" 정책 복제).
- OxiBrowser는 이미 oxibrowser-cdp를 갖고 있으므로: 사용자가 로컬 브라우저(또는 CDP 클라이언트)로 세션에 접속해 로그인 → 제어권 반납 → 에이전트 재개. 재개 시 "이전 상태에서 재개 or 재수립" 분기도 Operator 동작을 그대로 따름.
- 연구 배경의 Cloudflare 터널 인증, Tailscale 콘솔 승인이 정확히 이 패턴으로 무인화된다(1회 인계 → 상태 저장).

### 2) 프로파일 2층 구조: 영구 프로파일 + 읽기 전용 모드 — Steel Profiles / Browserbase Contexts
- named profile 디렉터리 + 종료 시 스냅샷 + `persist:false`(읽기 전용 재사용, 병렬 안전) + 프로파일별 스코프(namespace 상당).
- Browserbase의 저장 데이터 암호화(at rest)와 "사이트·로그인당 1컨테이너" 규칙, Steel의 "복원 후 로그인 상태 검증 → 실패 시 재인증 라우팅, 무한 재시도 금지"를 함께 채택.
- Skyvern의 프로파일 잠금 충돌 이슈(#4390)에서 배우는 동시 실행 잠금(advisory lock) 필수.

### 3) 자격증명 브로커 + 실행 시점 주입 — Steel Credentials API / Skyvern Bitwarden / browser-use sensitive_data
- 원천 자격증명은 프롬프트·모델 컨텍스트 밖(origin 스코프 저장소)에 두고, executor가 액션 실행 직전 필드에 주입. `exactOrigin`(지정 오리진에만), `blurFields`(캡처에서 블러), `autoSubmit:false`(제출 전 승인) 3종 제어 복제.
- OxiBrowser 실현形态: credential provider 트레잇 — macOS Keychain(`security find-generic-password`), Bitwarden CLI(`bw serve` REST, Skyvern의 브리지 구성 참고), env/파일 제공자. 연구 목표인 "키체인 재사용"의 직접적 구현체.
- browser-use의 `<secret>` 플레이스홀더 + 도메인별 스코프 + `allowed_domains` 병행 권고를 API 표면으로 차용.

### 4) 도메인 허용목록 + watch mode — OpenAI / browser-use allowed_domains
- 세션/프로파일에 허용 오리진 목록을 강제(인젝션된 페이지가 다른 도메인으로 자격증명·데이터 유출 시도 차단). OpenAI의 엔터프라이즈 도메인 차단(서브도메인 전체 `.example.com`) 표기법 참고.
- 민감 도메인에서는 watch mode 상당: 사용자 승인 세션에서만 동작, 타임아웃 시 자동 일시정지.

### 5) 실행 계층의 확인 게이트(바이팠스 불가) — OpenAI 고영향 확인 / Steel autoSubmit:false, browser-use 반면교사
- 위험 액션(제출, 결제, 삭제, 권한 변경, 메시지 전송, 파일 업로드, JS 실행)을 **executor에서 분류해 승인 대기 상태로 전환**. LLM 프롬프트로 "물어보라"고 지시하는 browser-use 방식은 모델이 우회할 수 있어 실패 사례로 명시돼 있다 — 반드시 실행 계층에서 차단.
- 승인 UX는 CDP 이벤트/알림으로 외부 에이전트(omp 등)에 전달하고, 대기 중 캡처는 계속하되 자격증명 필드는 블러(Steel blurFields).

---

## 5. 실행 가능한 권고

OxiBrowser(현재: crates/oxibrowser{,-cdp,-core,-render}, session/executor, HAR 캡처, 스킬 시스템) 기준 구체 권고. 우선순위 순.

1. **(P0) 프로파일 지속성 추가**: session 모듈에 `profile` 개념 도입. `~/.oxibrowser/profiles/<name>/`(user-data-dir 상당: 쿠키·localStorage 저장소) + 세션 종료 시 스냅샷 + `persist: false` 읽기 전용 모드 + 프로파일별 advisory lock. 쿠키 저장소는 브라우저 상태와 함께 at-rest 암호화(Browserbase Contexts 방식). 프로파일 복원 후에는 인증 페이지 검증(로그인 리디렉션 감지) 후 실패 시 재인증 경로로 라우팅.
2. **(P0) 실행 계층 확인 게이트**: executor에 위험 액션 분류기(폼 제출, 결제, 삭제, 권한 변경 등)와 `ApprovalPending` 상태 추가. 승인은 CDP 이벤트 또는 콜백으로 소비자(에이전트)에게 위임. HAR·스크린샷에서 자격증명 유형 필드는 기본 블러.
3. **(P1) 자격증명 공급자 인터페이스**: `CredentialProvider` 트레잇(keychain / bitwarden-cli / env / 파일). 조회는 origin 바인딩, executor가 입력 직전 주입, 모델 컨텍스트·로그·HAR에는 플레이스홀더만 노출. `auto_submit=false` 옵션으로 "채우기 후 제출 전 승인" 지원. 1차 대상은 macOS Keychain — 연구 배경의 무인 시나리오(서버 세팅 에이전트)를 직접 지원.
4. **(P1) takeover 모드**: 세션의 입력 채널을 사용자 전용으로 전환하는 모드. 그 구간 캡처(스크린샷·DOM·HAR) 자동 중단 + 민감 필드 마스킹, 제어권 반납 후 상태 재검증 후 재개. CDP 연결 기반이므로 구현 비용이 낮고 Operator UX의 검증된 복제품.
5. **(P2) 도메인 허용목록·차단**: 세션/프로파일 생성 시 `allowed_domains` 강제 옵션, 서브도메인 와일드카드 표기 지원. 자격증명 주입·프로파일 사용 시에는 기본 동작으로 켜기(browser-use의 공식 권고와 동일).
6. **(P2) 문서·운용 규칙 패키지화**: "동시 실행 금지, 사이트·계정당 1프로파일, 저장 상태는 민감 데이터 취급(파일 권한), 세션 만료·IP 바인딩 대응 안내"를 스킬/문서로 제공 — 모든 경쟁 제품이 동일하게 명시하는 운용 계약이며, Skyvern·browser-use의 공개 이슈가 실패 사례 집합.

비권고(근거): Web Bot Auth류의 원격 신원 증명은 호스팅 서비스용 표준이라 로컬 퍼스트인 OxiBrowser의 당면 과제가 아님. 컨테이너 VM 격리도 로컬 실행 모델과 맞지 않음 — 대신 프로파일 스코프+허용목록으로 동일 위험을 축소.

---

## 6. 주요 출처

1. OpenAI — Operator System Card: https://openai.com/index/operator-system-card/
2. OpenAI Help — ChatGPT agent (인계·쿠키·블록리스트): https://help.openai.com/en/articles/11752874-chatgpt-agent
3. OpenAI Help — Using cloud browser in ChatGPT (secure form): https://help.openai.com/en/articles/20001280-using-cloud-browser-in-chatgpt
4. OpenAI Developers — Sandbox security (VM/컨테이너·이그레스 권고): https://developers.openai.com/api/docs/guides/agents-api/environments/security
5. 1Password — AI 어시스트 브라우징 보안 어드바이저리: https://1password.com/blog/security-advisory-for-ai-assisted-browsing-with-the-1password-browser
6. Anthropic — computer use 발표: https://www.anthropic.com/news/3-5-models-and-computer-use
7. Anthropic Support — Claude for Chrome 시작 가이드: https://support.anthropic.com/zh-CN/articles/12012173-chrome%E7%89%88claude%E5%85%A5%E9%97%A8%E6%8C%87%E5%8D%97
8. Anthropic — Mitigating prompt injections in browser use: https://www.anthropic.com/research/prompt-injection-defenses
9. browser-use — 스킬/레퍼런스 문서(스토리지·프로파일·sensitive_data): https://github.com/browser-use/browser-use/blob/main/skills/open-source/references/browser.md · https://github.com/browser-use/browser-use/blob/main/examples/features/sensitive_data.py
10. Steel — Profiles API 개요: https://docs.steel.dev/overview/profiles-api/overview
11. Steel — Credentials API(비밀 주입): https://llms.steel.dev/articles/credentials-api-for-browser-agents/
12. Browserbase — Contexts 문서(암호화·무기한·운용 규칙): https://docs.browserbase.com/platform/browser/core-features/contexts
13. Browserbase — Website authentication 가이드(Live View·2FA): https://docs.browserbase.com/platform/identity/authentication
14. Browserbase — Identity & 1Password agentic autofill: https://www.browserbase.com/identity · https://www.browserbase.com/blog/1password-agentic-autofill
15. Stagehand — README·act(variables): https://github.com/browserbase/stagehand
16. Skyvern — credentials 문서(passwords·bitwarden): https://github.com/Skyvern-AI/skyvern/blob/main/fern/credentials/passwords.mdx · https://github.com/Skyvern-AI/skyvern/blob/main/fern/credentials/bitwarden.mdx
