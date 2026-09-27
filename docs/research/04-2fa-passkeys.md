# 4. 2FA / 패스키 / TOTP — 에이전트 무인 인증 재사용 조사

- 작성일: 2026-09-27
- 대상 시스템: OxiBrowser(v0.22, pure-Rust 헤드리스, boa JS 엔진, oxibrowser-cdp, session/executor)
- 문제 배경: 서버 세팅 에이전트가 ① Cloudflare 터널 인증(브라우저), ② Tailscale 관리콘솔 승인, ③ macOS iCloud 로그아웃, ④ 시스템설정 GUI 승인에서 사용자 손을 빌려야 했음. 목표는 "사용자 자격증명(키체인 등)을 안전하게 재사용해 무인 진행"하는 것.

---

## 요약 (TL;DR)

| 인증 수단 | 에이전트 무인 재사용 | 핵심 조건 |
|---|---|---|
| 소프트웨어 패스키(가상 인증기로 **에이전트가 직접 등록**) | ✅ 가능 | WebAuthn 가상 인증기 + 개인키 안전 저장. 서버엔 유효한 공개키만 존재하면 됨 |
| iCloud 키체인 동기화 패스키(**기존 것**) | ❌ 불가 | 비밀키 비노출, ASAuthorization 중개 + 사용자 제스처 + 연관 도메인 필요 → CLI/헤드리스 접근 불가 |
| TOTP(otpauth:// 시크릿 보유 시) | ✅ 가장 실용적 | 시크릿을 키체인에 저장, oathtool/totp-rs로 생성 |
| SMS/이메일 2FA | ❌ 자동화 금지 | out-of-band 채널 자체가 '사용자 소유'가 증명 대상. 자동화 구조는 피싱 릴레이와 동일 |
| step-up(재인증) 요구 | 조건부 | AAL/세션 만료 정책에 따라 사람 상승(human escalation) 설계 필요 |

---

## 1. WebAuthn 가상 인증기 (CDP / Playwright)

### 1.1 CDP WebAuthn 도메인

Chromium은 DevTools Protocol에 `WebAuthn` 도메인을 두고 소프트웨어 인증기를 시뮬레이션한다. 헤드리스에서도 동작하며, 실제 서버 검증을 통과하는 **암호학적으로 유효한** 등록/서명 응답을 만든다.

핵심 명령 시퀀스:

```jsonc
// 1) 도메인 활성화 (enableUI:false면 인증기 UI 없이 진행)
{ "method": "WebAuthn.enable", "params": { "enableUI": false } }

// 2) 가상 인증기 추가
{ "method": "WebAuthn.addVirtualAuthenticator", "params": { "options": {
    "protocol": "ctap2",
    "ctap2Version": "ctap2_1",
    "transport": "internal",
    "hasResidentKey": true,          // discovarable credential(=passkey) 지원
    "hasUserVerification": true,
    "isUserVerified": true,          // UV 성공으로 처리
    "automaticPresenceSimulation": true
}}}
// → { "authenticatorId": "..." }

// 3) 이후 명령/이벤트
//    WebAuthn.setAutomaticPresenceSimulation / setUserVerified
//    WebAuthn.addCredential / getCredentials / clearCredentials
//    WebAuthn.removeVirtualAuthenticator
//    이벤트: credentialAdded, credentialAsserted
```

`VirtualAuthenticatorOptions` 주요 필드: `protocol`(u2f/ctap2), `ctap2Version`, `transport`(usb/nfc/ble/cable/internal), `hasResidentKey`, `hasUserVerification`, `hasLargeBlob`, `hasPrf`, `automaticPresenceSimulation`, `isUserVerified`, `defaultBackupEligibility/State` 등.

- 출처: [CDP WebAuthn 도메인](https://chromedevtools.github.io/devtools-protocol/tot/WebAuthn/), [Chrome DevTools WebAuthn 에뮬레이션](https://developer.chrome.com/docs/devtools/webauthn)

**보안상 중요한 뉘앙스**:
- `hasUserVerification:true`는 "UV 지원 여부", `isUserVerified:true`는 "UV 성공 처리". 결정적 테스트를 위해 둘 다 필요.
- `WebAuthn.getCredentials`는 credential별 `privateKey`(base64)까지 반환한다. 즉 **한번 등록된 가상 인증기 credential은 덤프해서 영속화했다가, `WebAuthn.addCredential`으로 다른 세션/컨텍스트에 다시 심을 수 있다**. 이것이 "에이전트 전용 패스키" 재사용의 기반.
- 서버 입장에서 이 credential은 정상 WebAuthn 공개키. 사이트가 attestation을 강제 검사하지 않는 한(대부분 안 함) 차단되지 않는다.

### 1.2 Playwright 가상 인증기 API

Playwright v1.61부터 브라우저 공통 고수준 API(`BrowserContext.credentials`)가 추가됐고, 크로스브라우저로 동작한다. Chromium 세부 제어가 필요하면 `CDPSession`으로 위 1.1을 직접 쓴다.

```ts
// 등록 컨텍스트에서 패스키 생성 후 추출
await ctx.credentials.install();            // navigator.credentials.* 가로채기 활성화(필수, 사전 설치)
await page.getByRole('button', { name: /create passkey/i }).click();
const [passkey] = await ctx.credentials.get({ rpId: 'localhost' });
// passkey = { id, rpId, userHandle, privateKey(PKCS#8 DER, base64url), publicKey(SPKI DER) }
await fs.writeFile('passkey.json', JSON.stringify(passkey));

// 로그인 컨텍스트에 재주입
await loginCtx.credentials.create(passkey.rpId, {
  id: passkey.id, userHandle: passkey.userHandle,
  privateKey: passkey.privateKey, publicKey: passkey.publicKey,
});
await loginCtx.credentials.install();
```

- 출처: [Playwright Credentials API](https://playwright.dev/docs/api/class-credentials), [Playwright CDPSession](https://playwright.dev/docs/api/class-cdpsession), [Playwright 인증 가이드](https://playwright.dev/docs/auth)

CDP 저수준 버전(`WebAuthn.enable` → `addVirtualAuthenticator` → `getCredentials` → `addCredential`)도 동일 맥락에서 사용 가능하며, `credentialAdded`/`credentialAsserted` 이벤트로 `waitForTimeout` 대신 동기화하는 패턴이 권장된다.

### 1.3 헤드리스 패스키 로그인 실험에서 반복되는 실패 지점

커뮤니티/문서에서 정리되는 함정들(예: webauthn.io 데모 대상 실험, SimpleWebAuthn 논의):

1. **RP ID 불일치**: credential의 `rpId`는 origin의 유효 도메인과 일치해야 함(`http://localhost:3000` → `localhost`).
2. **origin 불일치**: `example.com`과 `staging.example.com`은 다른 WebAuthn 컨텍스트.
3. **UV 요구 사이트**: `hasUserVerification:true` + `setUserVerified(true)` 누락 시 실패.
4. **usernameless 로그인**: `hasResidentKey:true` 필요.
5. **컨텍스트 경계**: 가상 credential은 컨텍스트 생명주기를 따름. 재사용하려면 `getCredentials`로 덤프 → 새 컨텍스트에서 `addCredential`로 재주입.
6. **navigator.credentials.get() 행걸림**: `setAutomaticPresenceSimulation` 미설정이 원인.
7. 구형 headless-shell 대신 modern headless 사용 권장.

근본 제약: **기존 계정에 이미 등록된(다른 인증기의) 패스키는 재현 불가** — 대응 공개키의 개인키가 없기 때문. 무인화하려면 (a) 사람이 한 번 로그인해 계정 보안 설정에서 **에이전트용 패스키를 새로 등록**하게 하고, 그 개인키를 에이전트 금고에 보관하는 경로가 정석이다.

### 1.4 OxiBrowser 적용 지점

저장소 확인 결과(`grep -i 'webauthn|navigator.credentials|publickey-credential'` → 무일치, `oxibrowser-cdp/src/domains/`에 webauthn 없음): **현재 WebAuthn 지원 전무**. 구현 필요 조각:

1. `oxibrowser-core` JS 런타임(boa)에 `navigator.credentials.create/get` + `PublicKeyCredential` 인터페이스 구현 — 챌린지/origin/RP ID 해시/서명을 코어에서 생성.
2. `oxibrowser-cdp/src/domains/webauthn.rs` 신설 — `enable`, `addVirtualAuthenticator`, `addCredential`, `getCredentials`, `setUserVerified`, `setAutomaticPresenceSimulation`, `removeVirtualAuthenticator` + `credentialAdded/credentialAsserted` 이벤트. 기존 `target.rs`/`runtime.rs` 도메인 패턴을 그대로 따르면 됨.
3. 인증기 상태(credential 목록 + 개인키)는 `storage_state`(cookies/localStorage 영속화)와 동일한 세션 스냅샷 메커니즘으로 직렬화 — 단, **개인키는 별도 암호화(키체인 연동) 필수**.
4. ES256 서명엔 RustCrypto `p256`(`ecdsa` feature)로 충분. attestation은 `none`으로 시작.

---

## 2. TOTP — otpauth:// 저장·생성·키체인 통합

### 2.1 otpauth:// URI (Key Uri Format)

Google Authenticator가 확립한 프로비저닝 형식. QR코드의 실체가 이 URI다.

```
otpauth://totp/ISSUER:ACCOUNT?secret=BASE32&issuer=ISSUER&algorithm=SHA1&digits=6&period=30
```

- 필수: `secret`(Base32, A–Z/2–7, 패딩 생략). TOTP/HOTP 구분은 path(`totp`/`hotp`), HOTP는 `counter` 필수.
- 권장: `issuer`를 label 접두사와 쿼리 양쪽에 동일하게. 호환성 기본값은 `algorithm=SHA1`, `digits=6`, `period=30`(일부 구형 클라는 algorithm/digits 무시).
- **URI/QR 자체가 시크릿** — 소유자는 코드를 마음대로 생성할 수 있다. 에이전트가 이 값을 저장하면 2FA 요인을 사실상 단일 요인으로 만드므로 저장 보호가 전부다.
- 출처: [Key Uri Format(google-authenticator wiki)](https://github.com/google/google-authenticator/wiki/Key%20Uri-Format), [RFC 6238(TOTP)](https://datatracker.ietf.org/doc/html/rfc6238)

### 2.2 코드 생성

**CLI — oathtool** (검증된 사실상 표준):

```bash
oathtool --base32 --totp 'JBSWY3DPEHPK3PXP'          # 현재 6자리
oathtool -b --totp=SHA1 --digits=6 --time-step-size=30 'SECRET'
printf '%s\n' 'SECRET' | oathtool -b --totp -          # stdin으로 주면 히스토리/프로세스 목록 노출 방지
```

**Rust — totp-rs** (RFC 호환, `gen_secret` feature로 시크릿 생성도 가능):

```rust
use totp_rs::{Algorithm, Builder, Secret, Totp};
let totp: Totp = Builder::new()
    .with_secret(Secret::try_from_base32("JBSWY3DPEHPK3PXP")?)
    .with_algorithm(Algorithm::SHA1).with_digits(6)
    .with_step_duration(30).with_skew(1).build()?;
let code = totp.generate_current();
```

직접 조립 시: RustCrypto `hmac`+`sha1`+`data-encoding`(base32) 조합이면 ~30줄. OxiBrowser처럼 의존성 최소화가 원칙이면 이 경로도 실용적.

- 출처: [oathtool(1) 매뉴얼](https://manpages.debian.org/testing/oathtool/oathtool.1.en.html), [totp-rs](https://github.com/constantoine/totp-rs), [docs.rs/totp_rs](https://docs.rs/totp_rs/latest/totp_rs/struct.Secret.html)

**운용 디테일**: 코드 생성 직후 제출. 남은 유효창이 3–5초 미만이면 다음 창을 기다렸다 제출(경계에서 만료 코드 거부 방지). 재시도 시 rate-limit(보통 코드당 1회, N회 실패 시 잠금)에 주의.

### 2.3 macOS 키체인 통합

`security` CLI로 generic password 항목에 저장/조회(첫 접근 시 접근 확인 다이얼로그가 뜨고, 승인하면 해당 앱이 ACL에 등록됨):

```bash
# 저장 (-w만 쓰면 대화형 입력 → 셸 히스토리 미노출)
security add-generic-password -U -s "otp:cloudflare" -a "$USER" -w

# 조회 + 즉시 코드 생성 (한 줄 파이프라인)
oathtool -b --totp "$(security find-generic-password -s 'otp:cloudflare' -a "$USER" -w)"
```

- 값은 base32 시크릿만 또는 otpauth:// URI 전체. URI 전체 저장 시 파서가 issuer/algorithm/digits/period까지 복원해 구성 오류를 없앰(권장).
- `security(1)`은 `-T`로 접근 허용 앱(ACL)을 지정할 수 있어, 무인 실행 바이너리만 허용하고 나머지 접근은 확인창을 띄우는 준비가 가능하다. [INFERENCE: 매뉴얼 세부 옵션 미확인, `man security`로 확인 권장]
- 키체인 조회는 decrypt를 수반하며 권한 없는 호출엔 프롬프트가 발생한다(Apple Security 프레임워크 문서).
- 출처: [kSecClassGenericPassword](https://developer.apple.com/documentation/security/ksecclassgenericpassword), [Apple Keychain Items](https://developer.apple.com/documentation/security/keychain-items)

**대안 — 이미 패스워드 매니저를 쓰는 경우 CLI가 최단 경로**:

```bash
export BW_SESSION="$(bw unlock --raw)"
bw get totp 'GitHub'                                   # Bitwarden CLI
op read 'op://Production/GitHub/one-time password?attribute=otp'   # 1Password CLI
```

무인 자동화엔 1Password 서비스 계정 / Bitwarden API-key 세션이 적합하다. 이미 TOTP를 매니저에 넣어둔 사용자라면 시크릿 이관 없이 즉시 재사용 가능.
- 출처: [Bitwarden CLI](https://bitwarden.com/help/cli/), [1Password op read](https://developer.1password.com/docs/cli/reference/commands/read/), [1Password 스크립트 통합](https://developer.1password.com/docs/cli/secrets-scripts)

### 2.4 Tailscale / Cloudflare 시나리오 매핑

- **Cloudflare 터널 인증**: `cloudflared tunnel login`은 dash.cloudflare.com 브라우저 인증을 요구. 이메일+비밀번호+TOTP가 키체인에 있으면 OxiBrowser 세션에서 3단계 전부 무인 완료 가능(패스워드·otpauth 시크릿을 키체인에서 읽고 TOTP는 §2.2로 생성). 근본적으로는 API Token 기반이 더 안전하지만(범위 축소 가능), 브라우저 경로가 필요한 경우 위 조합이 답.
- **Tailscale 관리콘솔**: SSO+2FA가 있는 관리 콘솔 승인도 동일 패턴. 단, 콘솔 로그인이 step-up 재인증을 요구하면 §5 정책을 따라야 함. 반복 작업은 API key(auth key)로 우회하는 것이 정석.

---

## 3. iCloud 키체인 동기화 패스키 — 외부 에이전트 사용 제약

결론부터: **헤드리스/CLI 에이전트가 iCloud 키체인의 기존 패스키로 서명하는 공식 경로는 없다.**

구조적 이유:

1. **비밀키 비노출 원칙**: `ASAuthorizationPlatformPublicKeyCredentialProvider`는 iCloud 키체인에 저장된 키쌍으로 등록/어설션을 *중개*할 뿐, 앱에 개인키를 주지 않는다. 서명은 시스템이 수행한다. ([Apple 문서](https://developer.apple.com/documentation/authenticationservices/asauthorizationplatformpublickeycredentialprovider))
2. **연관 도메인 + AASA 필수**: 앱은 RP 도메인을 `webcredentials:` 연관 도메인으로 선언하고, 사이트 AASA 파일이 앱 Team ID/번들 ID와 연결돼야 인증이 동작한다. ([Supporting passkeys](https://developer.apple.com/documentation/AuthenticationServices/supporting-passkeys)) → 제3자 도메인(Tailscale, Cloudflare)용으로는 그 사이트가 에이전트 앱을 AASA에 등록해줘야 한다는 뜻으로, 현실적으로 불가능.
3. **브라우저 앱 경로도 사용자 승인 필요**: 다른 사이트의 패스키를 대행하는 브라우저는 `ASAuthorizationWebBrowserPublicKeyCredentialManager`를 쓰되, **사용자가 해당 브라우저에 패스키 사용을 1회 명시적으로 허가**해야 하고 GUI 사용자 제스처(Touch ID/암호)가 수반된다. ([Authenticating with passkeys in browser apps](https://developer.apple.com/documentation/authenticationservices/authenticating-people-by-using-passkeys-in-browser-apps), [Apple 포럼 확인답변](https://developer.apple.com/forums/thread/767028))
4. **내보내기 불가**: macOS 비밀번호 CSV 내보내기는 **비밀번호만** 대상. 패스키는 암호화되지 않은 파일로 내보내지지 않으며, 최신 OS의 앱 간 이전(credential exchange, `ASCredentialExportManager`)도 양쪽 앱이 Apple 교환 시스템을 구현한 경우에만 동작한다. ([Apple 지원 문서](https://support.apple.com/guide/iphone/export-passwords-iphf28f2e93e/27/ios/27), [ASCredentialExportManager](https://developer.apple.com/documentation/authenticationservices/ascredentialexportmanager))
5. 배포 요건: 연관 도메인 entitlement + 유효한 코드 서명(Developer ID 포함)이 전제. 시스템 설정의 "iCloud 로그아웃" 같은 단계는 사용자 세션 자체가 전제라 에이전트화 불가 — 이건 인증이 아니라 macOS 사용자 계정 정책 영역.

**실행 가능한 대안 3가지**(우선순위순):

1. **에이전트 전용 패스키 신규 등록**: 사용자가 마지막으로 한 번 직접 로그인해 계정 보안 설정에서 OxiBrowser 가상 인증기(§1)로 패스키 추가. 이후 무인 서명. iCloud와 무관한 독립 자격증명이 생김.
2. **패스키 지원 PM으로 재등록**: 1Password/Bitwarden 등 패스키 저장+CLI 노출이 되는 매니저에 패스키를 새로 만들고 CLI로 재사용(§2.3과 동일 아이디어).
3. **TOTP 병행**: 사이트가 패스키만 허용하지 않는 한, otpauth 시크릿 기반 TOTP가 구현 비용 대비 가장 높은 무인화 효율(§2).

---

## 4. SMS/이메일 2FA — 자동화하면 안 되는 이유와 인간 상승 설계

### 4.1 자동화 금지 근거

1. **NIST 분류**: SP 800-63B-4는 SMS/음성 OTP를 PSTN 기반 out-of-band 인증기로 **'restricted'** 분류. SIM 변경·기기 교체·번호 이동을 위험 지표로 점검하고, 비-제한 대안 수단 제공·위험 고지·이폰 계획을 요구한다. 피싱 내성도 없다. ([SP 800-63B-4](https://pages.nist.gov/800-63-4/sp800-63b.html))
2. **채널 소유가 증명 대상**: SMS 2FA의 검증 대상은 "등록된 전화번호/구독 제어". 그 채널은 사용자 신체 근처 기기로만 도달한다. 에이전트가 코드를 얻으려면 사용자로부터 코드를 *전달받아야* 하는데, 이 구조(실시간 릴레이)는 피싱 사이트가 희생자에게 요구하는 것과 정확히 동일한 위상이다. 자동화 순간 에이전트는 MITM이 된다.
3. **이메일 OTP도 마찬가지**: 코드 조회를 위한 사서함 접근 권한은 계정 탈취 blast radius를 메일 전체로 확대한다. 비밀번호 재설정 메일까지 노출된다.
4. **운영 리스크**: OTP 재시도 rate-limit, 로그인 시도 패턴 이상으로 계정 잠금, 서비스 ToS 위반(자동화 금지 조항) 소지.

### 4.2 인간 상승(human-in-the-loop) 전환 설계

원칙: **2FA 요인을 에이전트가 취득하지 않고, 해당 단계만 사용자에게 넘긴 뒤 세션을 이어받는다.**

상태머신 예(OxiBrowser session/executor에 붙이는 관점):

```
Agent: id/password 자동 입력(키체인)      ← 재사용 가능한 부분
  └─ 2FA 화면 감지 → state=AWAIT_HUMAN
       ├─ 체결(escalation) 생성: {사유, 대상 사이트, 타임아웃(기본 5분), 복구 불가 여부}
       ├─ 채널: (a) 제어 브라우저 화면을 사용자에게 노출(로컬 GUI/VNC)해 직접 입력
       │        (b) MCP elicitation input_required 로 "지금 사이트 X의 2FA를 직접 완료해 달라" 통지
       ├─ 사용자가 코드를 사이트에 직접 입력 → 에이전트는 로그인 완료(URL/DOM 변화)만 감지
       └─ 타임아웃 초과 → state=BLOCKED, 감사 로그 기록 후 안전 종료
  └─ 세션 쿠키는 storage_state로 영속화 → 이후 무인 재사용
```

설계 규칙:

- **코드는 절대 에이전트를 경유하지 않게 입력 UI를 사용자에게 제공**한다. 에이전트가 "코드를 보내달라"고 요청하는 순간 피싱 구조가 된다(§4.1-2).
- MCP 표준으로는 elicitation이 정합한 기제: 서버가 `elicitation/create`(form 모드, `approved` boolean 필드)로 승인/입력을 요청하고, 클라이언트는 `accept/decline/cancel`을 반환. 2026-07-28 개정판은 `input_required` 결과로 비상태적 전환. 명세는 "elicitation은 사용자 입력 요청이지 권한부여 체계 자체가 아니며" 표시할 정보(요청 행위·결과·대상·파괴성)와 서버 측 독립 검증을 요구한다. ([MCP 사양](https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation))
- 상승 요청은 **스코프 + 타임아웃 + 감사 로그**(누가/언제/무엇을 승인했는지)를 항상 포함. decline/cancel/누락 필드는 전부 거부로 처리(암묵 승인 금지).
- 2FA 완료 후 얻은 세션(쿠키)은 민감 자격증명. 저장 시 암호화하고 §5의 재인증 예산 안에서만 재사용.

---

## 5. step-up 인증 정책

### 5.1 표준 지형

- **NIST 세션 관리(SP 800-63B-4)**: overall timeout(재인증 상한) — AAL2 ≤ 24h(SHOULD), AAL3 ≤ 12h(SHALL). inactivity timeout — AAL2 ≤ 1h(SHOULD), AAL3 ≤ 15min(SHOULD). 재인증 성공 시 두 타이머 리셋. (구 800-63B-3의 12h/30분 수치와 혼동 주의.) ([Session Management](https://pages.nist.gov/800-63-4/sp800-63b/session/), [AAL](https://pages.nist.gov/800-63-4/sp800-63b/aal/))
- **OIDC 요청 파라미터**: `prompt=login`(강제 재인증), `max_age=<초>`(인증 신선도), `acr_values`(요구 인증 컨텍스트).
- **RFC 9470 (OAuth 2.0 Step-up Challenge, 2023-09)**: RS가 토큰 강도 부족 시 `401` + `WWW-Authenticate: Bearer error="insufficient_user_authentication", acr_values=..., max_age=...`로 재요구 → 클라이언트가 해당 파라미터로 재인가 요청 → 새 토큰의 `acr`/`auth_time` 클레임으로 검증. ([RFC 9470](https://www.rfc-editor.org/info/rfc9470/))
- 실제로 Cloudflare/Tailscale 등 관리 콘솔은 민감 변경 시 "최근 로그인" 요구(사실상 max_age step-up)를 걸어두는 경우가 많다. 이는 §4 인간 상승과 동일한 트리거로 취급해야 한다.

### 5.2 에이전트 함의

1. **재인증은 불가피한 이벤트로 모델링**: 세션 쿠키/storage_state는 유한 수명. "재인증 필요" 감지(로그인 폼 재출현, 401/redirect to /login)를 executor의 명시적 상태로 두고, TOTP/패스키 재생을 자동 시도 → 실패 시 인간 상승.
2. **재인증 예산**: 사이트별로 성공한 재인증 수단(TOTP 여부, 패스키 여부)과 마지막 인증 시각을 세션 메타데이터로 추적. AAL2급 사이트는 최소 1시간 이내 비활성 만료를 가정해 긴 작업은 세션 체크포인트 후 재개.
3. **step-up 응답 파싱**: API 경로에서 `WWW-Authenticate ... insufficient_user_authentication`을 만나면 재시도 금지하고 상승 플로우로. 무한 재시도는 계정 잠금 경로다.
4. **권한 최소화**: 장기 무인에는 세션 재사용보다 범위 축소된 API 토큰(Cloudflare API Token, Tailscale auth key 등)이 정답인 경우가 많다. 브라우저 2FA 자동화는 "API가 없을 때의 최후 수단"으로 위치づ켜야 한다.

---

## 실행 가능한 권고

OxiBrowser 기준 우선순위:

1. **[즉시, 저비용] TOTP 파이프라인 확보** — macOS 키체인에 otpauth:// URI를 generic password로 저장(`security add-generic-password -U -s "otp:<service>" -w`)하고, `oxibrowser` CLI에 `--totp-from-keychain <service>` 옵션 추가. 구현은 RustCrypto `hmac`+`sha1`+`data-encoding`(또는 `totp-rs`)으로 코어 내장, 외부 `oathtool` 의존 없음. 유효창 잔여 3초 미만이면 다음 창 대기. 이것만으로 Cloudflare 터널 인증류의 대부분이 무인화된다.
2. **[즉시] 사용자 자격증명 읽기 경로 표준화** — 비밀번호·otpauth 시크릿 조회를 `security find-generic-password -w`(또는 `op read`/`bw get`)로 통합하고, 조회값을 메모리에서만 다루며 로그/HAR 캡처에서 마스킹. HAR 캡처가 있는 만큼 **2FA 폼 필드·OTP 값은 HAR에서 자동 레드랙트**하는 규칙을 지금 넣을 것.
3. **[단기] WebAuthn 가상 인증기 구현** — `oxibrowser-cdp`에 `WebAuthn` 도메인 추가(§1.4), 코어 JS에 `navigator.credentials.create/get`(ES256, attestation `none`) 구현. 온보딩 플로우: "사용자가 1회 로그인 → 사이트 보안 설정에서 OxiBrowser 패스키 등록 → 개인키를 키체인 항목으로 저장 → 이후 무인 서명". 기존 iCloud 패스키 재사용은 불가하므로 시도조차 하지 말 것(§3).
4. **[단기] 인간 상승 프리미티브** — executor에 `AWAIT_HUMAN` 상태 + 타임아웃 + 감사 로그 추가. MCP 클라이언트가 붙는 `oxibrowser mcp` 경로에 elicitation/input_required 연결. 원칙: SMS/이메일 코드는 에이전트 경유 금지, 사용자가 화면에 직접 입력(§4.2). iCloud 로그아웃·시스템설정 GUI 승인은 이 상태로 항상 분류.
5. **[정책] step-up·세션 만료 취급** — 재인증 요구를 명시적 이벤트로 모델링하고, API 토큰 경로(Cloudflare API Token, Tailscale auth key)가 있으면 브라우저 2FA보다 우선. 서비스 ToS·계정 잠금 리스크 때문에 무인 로그인 재시도는 지수 백오프 + 상한(예: 3회).
6. **[보안] 저장 강도** — TOTP 시크릿·패스키 개인키는 전부 키체인(또는 암호화된 세션 금고)에. 평문 파일/스토리지_state 직렬본에 넣지 않기. 이 값들은 "2FA 우회 열쇠"이므로 누출 시 2FA가 무의미해진다.

---

## 주요 출처

1. CDP WebAuthn 도메인 — https://chromedevtools.github.io/devtools-protocol/tot/WebAuthn/
2. Chrome Devtools WebAuthn 에뮬레이션 — https://developer.chrome.com/docs/devtools/webauthn
3. Playwright Credentials API(v1.61~) — https://playwright.dev/docs/api/class-credentials
4. Playwright CDPSession — https://playwright.dev/docs/api/class-cdpsession
5. Google Authenticator Key Uri Format — https://github.com/google/google-authenticator/wiki/Key-Uri-Format
6. RFC 6238(TOTP) — https://datatracker.ietf.org/doc/html/rfc6238
7. oathtool(1) — https://manpages.debian.org/testing/oathtool/oathtool.1.en.html
8. totp-rs — https://github.com/constantoine/totp-rs
9. NIST SP 800-63B-4 — https://pages.nist.gov/800-63-4/sp800-63b.html · 세션: https://pages.nist.gov/800-63-4/sp800-63b/session/ · AAL: https://pages.nist.gov/800-63-4/sp800-63b/aal/
10. Apple, Supporting passkeys — https://developer.apple.com/documentation/AuthenticationServices/supporting-passkeys
11. Apple, Passkeys in browser apps — https://developer.apple.com/documentation/authenticationservices/authenticating-people-by-using-passkeys-in-browser-apps
12. Apple, 비밀번호 내보내기(패스키 제외) — https://support.apple.com/guide/iphone/export-passwords-iphf28f2e93e/27/ios/27
13. Apple, ASCredentialExportManager — https://developer.apple.com/documentation/authenticationservices/ascredentialexportmanager
14. RFC 9470(OAuth Step-up Challenge) — https://www.rfc-editor.org/info/rfc9470/
15. MCP Elicitation 사양 — https://modelcontextprotocol.io/specification/2025-06-18/client/elicitation

보조: Bitwarden CLI(https://bitwarden.com/help/cli/), 1Password `op read`(https://developer.1password.com/docs/cli/reference/commands/read/), Apple kSecClassGenericPassword(https://developer.apple.com/documentation/security/ksecclassgenericpassword), Apple 포럼 패스키 배포 확인(https://developer.apple.com/forums/thread/767028)
내부 증거: OxiBrowser 저장소 `oxibrowser-cdp/src/domains/`(webauthn 부재), 전체 grep `webauthn|navigator.credentials` 무일치(2026-09-27 확인)
