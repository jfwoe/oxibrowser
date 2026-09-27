# macOS 키체인/시크릿 저장 — 에이전트 무인 자격증명 재사용 조사

작성일: 2026-09-27 · 대상: OxiBrowser(v0.22, pure-Rust 헤드리스) 기반 서버 세팅 에이전트
배경 시나리오: Cloudflare 터널 인증(브라우저), Tailscale 관리콘솔 승인, iCloud 로그아웃, 시스템설정 GUI 승인 단계에서 사용자 개입이 발생 → 에이전트가 사용자 자격증명을 안전하게 재사용해 무인 진행하는 구조 조사.

---

## 요약 (TL;DR)

1. macOS 키체인은 **API 3종(Keychain/SecKeychain/SecItem) × 구현 2종(file-based/data-protection)** 구조다. CLI/비번들 Rust 프로세스가 접근하는 `login.keychain-db`는 **file-based**이며, 접근 통제는 **ACL(신뢰 앱 목록) + partition ID(코드서명 게이트)** 2층으로 동작한다.
2. macOS 26 Passwords 앱은 **iCloud 키체인(데이터 보호 키체인)의 UI**이며, 제3자가 그 내부 DB를 읽고 쓰는 **공개 API는 없다**. Apple이 제공하는 통로는 AuthenticationServices의 credential provider/import-export뿐 — 에이전트가 "Passwords 앱에서 비밀번호 꺼내기"는 불가능하고, 로컬 키체인·브로커 CLI 경로를 써야 한다.
3. 가장 실용적인 무인 경로는 **에이전트 → 시크릿 브로커(1Password CLI `op` / Bitwarden `bw`·`rbw`) → 저장소** 또는 **에이전트 → `keyring`/`apple-native-keyring-store`(Rust) → 로컬 키체인**이다. 1Password는 데스크톱 앱 연동(Touch ID 승인, XPC + 코드서명 상호 검증)과 서비스 어카운트(무인 전용 볼트)를 모두 제공해 상황별 분기가 깔끔하다.
4. 권고: OxiBrowser에 **시크릿 브로커 계층**을 두고, per-site/per-agent 서비스 네이밍(`com.oxibrowser.agent/<agent>/<site>/<kind>`)으로 키체인 아이템을 격리하며, 자격증명은 **단일 문자열이 아니라 JSON 레코드(otpauth:// URI 포함)**로 저장한다.

---

## 1. macOS 키체인 접근 모델

### 1.1 두 개의 키체인 구현과 세 개의 API

Apple TN3137이 이 구조의 정본이다 ([TN3137: On Mac keychain APIs and implementations](https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains)).

| 구분 | file-based keychain | data protection keychain |
|---|---|---|
| 기원 | 클래식 Mac OS | iOS, macOS 10.9에서 iCloud 키체인과 함께 이식 |
| 물리 형태 | `~/Library/Keychains/login.keychain-db` 등 **파일** | 사용자당 정확히 1개, 시스템이 컨텍스트로 선택 |
| 접근 통제 | **ACL(SecAccess)** — 신뢰 앱 목록 + partition ID | **keychain access group**(엔타이틀먼트 기반) + 선택적 SecAccessControl |
| iCloud 동기화 | 불가 | `kSecAttrSynchronizable` 설정 시 가능 |
| Touch ID/Secure Enclave 보호 | 불가(SecAccessControl 미지원) | 가능 |
| 실행 컨텍스트 | 사용자 + launchd 데몬 모두 | **사용자 로그인 컨텍스트만** |
| CLI 도구 | `security` CLI의 주 타깃 | 사실상 지원 안 함 |

핵심 규칙:
- SecItem API는 기본적으로 file-based를 타깃한다. 데이터 보호 키체인으로 가려면 `kSecUseDataProtectionKeychain=true` 또는 `kSecAttrSynchronizable=true`가 필요하다.
- 데이터 보호 키체인의 access group은 **코드서명 엔타이틀먼트 + 프로비저닝 프로파일**로 결정된다. 번들 구조가 없는 CLI/Rust 바이너리는 사실상 접근할 수 없다(TN3137은 daemon-with-restricted-entitlement 래핑을 안내하지만 여전히 사용자 컨텍스트에서만 동작).
- **file-based는 "폐기 예정 로드(deprecation road) 위"이나 공식 deprecated는 아니다.** iCloud 키체인·생체인증 등 신기능은 전부 데이터 보호 키체인 전용이다.

→ 에이전트(특히 비번들 Rust 바이너리, 잠재적으로 launchd 서비스)가 접근 가능한 영역은 **file-based login 키체인**뿐이라는 것이 설계 출발점이다.

### 1.2 file-based 키체인의 2층 접근 통제: ACL + partition ID

**일반 ACL(신뢰 앱 목록).** 각 키체인 아이템의 access 객체는 ACL 엔트리들로 구성되고, 엔트리는 (허용 연산: decrypt/sign/…, 신뢰 앱 목록, 프롬프트 정책)을 갖는다. 호출 앱이 신뢰 목록에 없으면 `securityd`가 SecurityAgent로 **Allow / Deny / Always Allow** 대화상자를 띄운다 ([Access Control Lists](https://developer.apple.com/documentation/security/access-control-lists)). "Always Allow"를 선택하면 그 앱이 해당 연산의 ACL에 영구 추가된다 — 이것이 "항상 허용" 지속화의 실체다.

중요: 신뢰 앱 판정은 경로가 아니라 **코드 서명의 designated requirement**로 이뤄진다 ([TN2206: macOS Code Signing In Depth](https://developer.apple.com/library/archive/technotes/tn2206/)). 따라서:
- ad-hoc 서명(재빌드 시마다 서명 변동) 또는 무서명 바이너리는 같은 "앱"으로 취급되지 않아 **매번 프롬프트가 재발**한다.
- Rust 에이전트를 키체인에 안정적으로 등록하려면 **안정적인 서명 신원**(Developer ID 또는 최소한 고정 ad-hoc identity + 고정 경로)이 필요하다.

**partition ID(별도의 코드서명 게이트).** `kSecACLAuthorizationPartitionID`는 ACL과 별개로 securityd가 관리하는 추가 인증 계층이다 ([ACL Authorization Keys](https://developer.apple.com/documentation/security/acl-authorization-keys)). 대표 값:
- `apple:` — Apple 서명 코드(`codesign`으로 키 사용 시 필수)
- `apple-tool:` — `/usr/bin/security` 등 Apple CLI
- `teamid:XXXXXXX` — 해당 팀 서명 앱
- `unsigned:` — 무/ad-hoc 서명 호출자(개발 중 흔히 발견됨)

일반 ACL에 "모든 앱"이 있어도 partition 게이트가 막히면 프롬프트가 계속된다. 에이전트가 키체인 프롬프트에 무한히 걸린다면 십중팔구 이 층이다. 진단: `security dump-keychain -a` 출력에서 `authorizations (1): partition_id` 확인 → 호출자 `codesign -dvvv` 결과와 대조.

### 1.3 `security` CLI 실무

`security(1)`은 file-based 키체인 조작의 기준 도구다(본 기기 man page에서 명령 존재 확인):

```bash
# 쓰기/읽기/삭제 (generic password)
security add-generic-password -s "com.oxibrowser.agent" -a "cloudflare" -w "SECRET" -U
security find-generic-password -s "com.oxibrowser.agent" -a "cloudflare" -w
security delete-generic-password -s "com.oxibrowser.agent" -a "cloudflare"

# 조회(비밀번호 출력 없이 메타데이터만)
security find-generic-password -s "com.oxibrowser.agent"

# ACL/partition 점검
security dump-keychain -a ~/Library/Keychains/login.keychain-db

# partition list 변경 — 키체인 비밀번호 필수(-k)
security set-generic-password-partition-list \
  -S 'apple-tool:,apple:,teamid:MYTEAMID' \
  -s 'com.oxibrowser.agent' -a 'cloudflare' \
  -k "$LOGIN_PASSWORD"
```

`set-key-partition-list` / `set-generic-password-partition-list` / `set-internet-password-partition-list` 모두 `-k`(키체인 비밀번호)를 요구한다 — 즉 **partition 변경은 무인으로 불가능하며 사용자 비밀번호 1회 입력이 필요한 관리자 작업**이다. 이는 에이전트 온보딩 단계에서 "한 번만 사람이 하는" 설치 절차로 설계하는 게 정답이다. 명령 구현 출처: [apple-oss-distributions/Security — SecurityTool/security.c](https://github.com/apple-oss-distributions/Security/blob/main/SecurityTool/macOS/security.c).

### 1.4 Rust 경로: `keyring` / `apple-native-keyring-store`

현재 생태계(2026-09 기준, crates.io 확인):
- `keyring` **4.2.0** — 두 모드: `v1` 피처(플랫폼 공통 `Entry::new(service, user)` 간편 API), `cli` 피처(모든 저장소 접속 글루). 앱이 저장소를 직접 고르려면 `keyring`이 아니라 `keyring-core` + 개별 저장소 크레이트 조합을 권장 ([docs.rs/keyring](https://docs.rs/keyring/latest/keyring/)).
- `apple-native-keyring-store` **1.0.2** — macOS 네이티브 저장소. `keychain`(전통 file-based, CLI/비프로비저닝 앱용)과 `protected`(데이터 보호 키체인, 번들+엔타이틀먼트 필요) 두 백엔드 제공 ([docs.rs/apple-native-keyring-store](https://docs.rs/apple-native-keyring-store/latest/apple_native_keyring_store/)).
- 저수준 직접 접근은 `security-framework` 3.7.0(SecItem 래퍼).

```toml
[dependencies]
keyring = { version = "4", features = ["v1"] }   # 간편 경로
```

```rust
use keyring::Entry;
let entry = Entry::new("com.oxibrowser.agent/srv1/cloudflare", "api-token")?;
entry.set_password("...")?;          // kSecClassGenericPassword 생성 (kSecAttrService/kSecAttrAccount 매핑)
let secret = entry.get_password()?;  // 권한 있으면 무프롬프트, 없으면 macOS가 프롬프트
```

필드 매핑: service→`kSecAttrService`, user→`kSecAttrAccount`, secret→`kSecValueData`, 아이템 클래스→`kSecClassGenericPassword`. `get_password()` 자체는 프롬프트를 만들지 않는다 — 프롬프트 여부는 전적으로 macOS 측(ACL/partition/잠김 상태)이 결정한다. 에이전트가 만든 아이템을 에이전트가 다시 읽는 한(작성자는 ACL의 소유자) 프롬프트 없이 재사용 가능하다. 이것이 "최초 1회 사용자 입력 → 이후 무인" 패턴의 기반.

### 1.5 Touch ID 프롬프트 UX와 "항상 허용" 지속화

두 메커니즘을 분리해야 한다:

| 목적 | 메커니즘 | 인증 수단 |
|---|---|---|
| 기존 아이템의 신뢰 앱 목록에 앱 추가 | file-based ACL 갱신 ("Always Allow") | **키체인(로그인) 비밀번호** |
| 아이템 사용 자체에 사용자 실재 요구 | `SecAccessControl`(데이터 보호 키체인 전용) | Touch ID/패스코드 |

- macOS 10.13.1부터 **ACL 변경 프롬프트는 셀렉터와 무관하게 항상 키체인 비밀번호를 요구**한다 — Touch ID로 대체 불가 ([SecACLCreateWithSimpleContents 문서](https://developer.apple.com/documentation/security/secaclcreatewithsimplecontents(_:_:_:_:_:))). Touch ID가 되는 건 `SecAccessControl` 쪽(`kSecAccessControlUserPresence`, `biometryCurrentSet` 등, [Restricting keychain item accessibility](https://developer.apple.com/documentation/Security/restricting-keychain-item-accessibility))이고, 이건 데이터 보호 키체인 전용이라 CLI 에이전트가 login 키체인에 적용할 수 없다.
- UX 설계 귀결: **무인 에이전트가 쓸 아이템은 Touch ID 게이트를 걸지 않는다.** Touch ID 프롬프트는 "사람이 승인하는 순간"(예: 브로커 언락)에만 두고, 에이전트가 반복 읽는 아이템은 안정 서명 + 최초 1회 ACL 승인으로 무프롬프트를 만든다. 민감도가 높으면 그 아이템을 아예 전용 키체인/전용 볼트로 격리한다.

---

## 2. macOS 26 Passwords 앱 · iCloud 키체인 vs 로컬 키체인

### 2.1 Passwords 앱과 공개 API의 실상

- Passwords 앱(macOS 15 Sequoia 도입, macOS 26에서 비밀번호 히스토리 등 추가 — [Apple Newsroom, macOS Tahoe 26](https://www.apple.com/newsroom/2025/06/macos-tahoe-26-makes-the-mac-more-capable-productive-and-intelligent-than-ever/))은 **iCloud 키체인(데이터 보호 키체인)의 사용자용 UI**다. 웹/앱 비밀번호·패스키·인증코드·Wi-Fi 비밀번호를 보여준다 ([Apple Support — Passwords 앱 가이드](https://support.apple.com/en-gb/120758)).
- **제3자가 Passwords 앱의 내부 DB를 읽거나 쓰는 공개 API는 존재하지 않는다.** Apple이 제공하는 통합점은 AuthenticationServices의 AutoFill credential provider(제3자 비밀번호 관리자가 시스템에 자격증명을 *제공*하는 방향)와 `ASCredentialImportManager`/`ASCredentialExportManager`(관리자 간 이전)뿐이다 ([ASCredentialProviderViewController](https://developer.apple.com/documentation/authenticationservices/ascredentialproviderviewcontroller)). 즉 "agent → Passwords 앱에서 꺼내기"는 플랫폼에서 금지된 방향이며, 앱이 자격증명을 소비하려면 `ASAuthorizationPasswordProvider` 기반 AutoFill 흐름(사용자 개입 수반)뿐이다.
- macOS 26에서도 **Keychain Access 앱은 존속**한다(인증서·키·legacy 키체인·ACL 관리 담당). 다만 비밀번호 내보내기는 Keychain Access에서 불가하고 Passwords로 이관됐다 ([Apple Support — Keychain Access 가이드](https://support.apple.com/en-ie/guide/keychain-access/kychn001/mac)).

### 2.2 iCloud 키체인 vs 로컬 login 키체인

| 항목 | 로컬 login 키체인(file-based) | iCloud 키체인(데이터 보호) |
|---|---|---|
| 웹/앱 비밀번호·패스키·인증코드 | 일부 로컬 복사본 존재 가능 | 동기화됨(E2E 암호화) |
| 인증서·개인키·SSH 키·앱 토큰·보안 메모 | **대부분 여기에만** | 원칙적 비동기 |
| 접근 주체 | 어떤 프로세스든(ACL 승인 하에) | 엔타이틀먼트 보유 번들 앱 |
| CLI 접근 | `security`/Rust로 가능 | 사실상 불가 |

- 동기화는 `kSecAttrSynchronizable`을 명시적으로 설정한 아이템만 된다. 제3자 앱이 만든 키체인 아이템은 기본적으로 로컬에 남는다 ([kSecAttrSynchronizable](https://developer.apple.com/documentation/security/ksecattrsynchronizable)).
- iCloud 키체인은 기기 간 신뢰 집합(sync circle)을 구성해 E2E 암호화로 동기화하며, 새 기기는 기존 기기 승인/복구 코드로 참여한다 ([Apple Platform Security Guide](https://help.apple.com/pdf/security/en_US/apple-platform-security-guide.pdf), [iCloud Keychain 설정](https://support.apple.com/en-ca/109016)).
- 개념적 포함관계: **Passwords 앱 ⊂ iCloud 동기화 자격증명 ⊂ macOS 전체 키체인 아이템**.

→ 에이전트 설계 함의: (a) 사용자의 *웹 비밀번호*를 무인으로 재사용하려는 접근은 Passwords/iCloud 쪽이 아니라 **자체 저장 아이템(사용자가 한 번 공급) 또는 브로커 볼트**로 해야 한다. (b) 사용자의 기존 Safari 비밀번호를 재사용하고 싶다면 공식 경로가 없으므로 **수출/이관(1Password 등으로 import)** 후 브로커 경로로 가는 것이 유일한 깨끗한 방법이다.

---

## 3. 서드파티 시크릿 브로커 (에이전트 친화적 경로)

### 3.1 1Password CLI (`op`)

가장 완성도 높은 2-트랙 모델 ([1Password CLI — Load secrets into scripts](https://developer.1password.com/docs/cli/secrets-scripts)):

**트랙 1 — 사람 감독 하 로컬 에이전트: 데스크톱 앱 연동 + Touch ID.**
- 설정: 1Password 앱 → Settings → Developer → "Integrate with 1Password CLI" 켜기. CLI 승인 요청 시 앱이 Touch ID/Apple Watch/Mac 비밀번호 프롬프트를 낸다 ([app integration 문서](https://developer.1password.com/docs/cli/app-integration)).
- 보안 구조: macOS에서 CLI↔앱은 **XPC로 통신하고 양측이 상호 코드서명 검증**. 승인은 계정별·터미널 세션별로 부여되고 **10분 비활성 만료(최대 12시간)**, 앱 잠금 시 즉시 철회된다 ([app integration security](https://developer.1password.com/docs/cli/app-integration-security)). — "무인"보다는 "사람 승인 1회 → 짧은 무인 윈도우"에 적합.
- 소비 패턴: `op read 'op://Vault/Item/field'` (단일 값) 또는 `.env`의 `op://` 레퍼런스를 `op run --env-file=.env -- ./agent`로 자식 프로세스 환경에 주입. 레퍼런스만 버전관리되고 비밀값은 디스크·프롬프트·로그에 안 남는다. 단 출력 마스킹은 보장이 아니므로(besti effort) 에이전트 출력 로깅 주의.

**트랙 2 — 자율 에이전트: 서비스 어카운트.**
- 전용 볼트(예: `Agents/srv-setup`)를 만들고, 그 볼트만 접근 가능한 서비스 어카운트 토큰을 `OP_SERVICE_ACCOUNT_TOKEN`으로 주입해 `op run` 사용. 개인 볼트·복구 정보와 완전 분리. 토큰 자체는 고가치 베어러 크레덴셜이므로 키체인에 보관하고 기체/프로젝트 변경 시 로테이션.
- MCP/SDK 통합으로 `op read` 임의 실행이 아니라 `get_credential(site, kind)` 같은 좁은 연산만 모델에 노출하는 것이 권장 패턴이다 ([1Password Developer Security](https://1password.com/developer-security)).

### 3.2 Bitwarden CLI (`bw`)와 `rbw`

**공식 `bw`** ([Bitwarden CLI 문서](https://bitwarden.com/help/cli/)):
- 스테이트리스 모델: `bw login`(이메일+마스터 비번 / `--apikey` / `--sso`) → `bw unlock`이 **세션 키** 발급 → `BW_SESSION` 환경변수 또는 `--session` 옵션으로 데이터 조작(`list`, `get`, `create`, `edit`). 세션은 터미널마다 재발급 필요, `bw lock`으로 무효화.
- 자동화 편의: `BW_CLIENTID`/`BW_CLIENTSECRET` 환경변수(API키 로그인), `bw unlock --passwordenv`/`--passwordfile`(마스터 비번 비대화식 제공 — 보관처는 사용자 책임, 곧 키체인).
- **`bw serve`**: 로컬 REST 서버(기본 `localhost:8087`)로 CLI 전 기능을 HTTP API로 노출. 기본적으로 `Origin` 헤더 있는 요청 차단(DNS rebinding 방어), `--hostname` 바인딩 주의. 에이전트가 HTTP로 자격증명을 조회하는 브로커 인터페이스로 쓸 수 있다 ([vault management API](https://bitwarden.com/help/vault-management-api/)).
- 한계: 무인 잔여 — 세션 키 관리가 번거롭고, 공식 서버가 CLI 트래픽을 봇으로 오탐하는 케이스 존재.

**`rbw`(비공식 Rust CLI)** ([github.com/doy/rbw](https://github.com/doy/rbw)):
- `rbw-agent` 백그라운드 데몬이 키를 메모리에 유지(ssh-agent/gpg-agent 방식) — `rbw get <name>`, `--field`, `--raw`(JSON)을 **세션 키 관리 없이** 사용. `lock_timeout`(기본 3600초) 후 자동 재잠금, `sync_interval`로 서버 동기화, pinentry로 프롬프트, 프로필(`RBW_PROFILE`)로 볼트 분리, 내장 SSH agent까지 포함.
- OxiBrowser와 같은 Rust 도구 생태계에서 "에이전트 데몬 + 얇은 클라이언트" 모델의 좋은 참고 구현.

### 3.3 비교

| | 로컬 키체인 + keyring | 1Password `op` | Bitwarden `bw`/`rbw` |
|---|---|---|---|
| 추가 설치 | 불필요(OS 내장) | 앱+CLI | CLI만 |
| 무인 잠금 해제 | 최초 ACL 승인 후 무프롬프트 | 서비스 어카운트(무인) / Touch ID(감독) | 세션키/API키/`--passwordfile` |
| 세분 권한 | 아이템 단위(service/account 네이밍으로 자체 관리) | **볼트+서비스어카운트 단위 격리** | 폴더/컬렉션 단위 |
| 감사 | 없음(로컬) | 계정 감사 로그 | 조직 이벤트 로그 |
| 동기화/백업 | 없음(기기 로컬) | 있음 | 있음 |
| otpauth·패스키·TOTP 필드 | 직접 인코딩 | 네이티트 필드 | 네이티브 필드 |

---

## 4. 권장 아키텍처: 에이전트 → 시크릿 브로커 → 키체인

### 4.1 계층 구조

```
에이전트(서버 세팅 / OxiBrowser 스킬)
   │  get_credential(site, kind) — 좁은 인터페이스, secret 참조만 취급
   ▼
시크릿 브로커 계층 (신뢰 경계)
   │  - 백엔드 선택: 로컬 키체인 | 1Password | Bitwarden
   │  - per-agent/per-site 격리 · 감사 로그 · 세션 수명 관리
   ▼
저장소: login 키체인(file-based) / 1Password 볼트 / Bitwarden 볼트
   ▲
최초 1회 온보딩(사람): 자격증명 공급 + ACL/partition 승인 또는 브로커 언락
```

설계 원칙:
1. **에이전트 프로세스에 raw secret을 환경변수로 흘리지 않는다.** 필요 시점(폼 필 입력, Authorization 헤더 세팅 직전)에 브로커에서 조회해 메모리에만 두고 즉시 폐기. OxiBrowser라면 CDP `Input`/DOM 입력 경로로 폼에 주입하고, HAR 캡처(기존 기능)에서 해당 값이 새지 않는지 **레드랙션 필터**를 두는 것이 선행 과제다.
2. **에이전트 CLI/바이너리는 안정 서명 신원을 갖는다.** 재빌드마다 ad-hoc 서명이 바뀌면 키체인 ACL이 무효화되어 프롬프트가 재발한다. Developer ID 서명(또는 고정 신원) + 고정 설치 경로.
3. **온보딩 절차를 "1회성 관리 작업"으로 분리.** 사용자가 (a) 자격증명을 브로커에 공급, (b) 키체인 ACL "항상 허용" 승인(키체인 비밀번호 1회), (c) 브로커 언락 정책 설정. 이후 무인.
4. **감독/자율 분기**: 사람이 자리에 있을 땐 Touch ID 승인(1Password 앱 연동 등)으로 배타적 승인을 얻고, 무인 런은 전용 서비스 어카운트/전용 볼트로만. 배경 시나리오의 "Tailscale 관리콘솔 승인"류는 무인이 아니라 **승인 요청 알림 → 사용자 원클릭 승인** 흐름(사람-인-더-루프)이 오히려 올바른 설계다.

### 4.2 per-site/per-agent 격리 — 저장 포맷

**서비스 네이밍(로컬 키체인 경로).** `kSecAttrService`/`kSecAttrAccount`를 구조화된 키로 쓴다:

```
service = com.oxibrowser.agent/<agent-id>/<site>
account = <login-id 또는 kind:password|totp|api-key|cookie>
```

- agent별 격리: `agent-id`가 다르면 아이템 공간 자체가 분리. 특정 에이전트 철회 시 `security delete-generic-password -s "com.oxibrowser.agent/srv1/..."`로 일괄 삭제 가능.
- 더 강한 격리(파일 단위): `security create-keychain oxibroker-agent-srv1.keychain-db`로 **전용 키체인 파일**을 만들고 `security set-keychain-settings -t 3600`(자동 잠금), 브로커만 서치리스트에 추가/제외. 민감 에이전트별로 잠금 정책·수명을 독립 운영할 수 있다.
- 1Password/Bitwarden 경로: 에이전트(또는 에이전트 클래스)마다 전용 볼트/컬렉션 + 서비스 어카운트/프로필 매핑(`rbw`의 `RBW_PROFILE` 참고).

**레코드 포맷.** 비밀번호 단일 문자열 대신 JSON 하나를 `kSecValueData`에 저장해 username·TOTP·메타를 원자적으로 관리:

```json
{
  "version": 1,
  "site": "https://dash.cloudflare.com",
  "kind": "login",
  "username": "user@example.com",
  "password": "…",
  "otp": "otpauth://totp/Cloudflare:user@example.com?secret=JBSWY3DPEHPK3PXP&issuer=Cloudflare",
  "notes": "터널: srv1-tunnel",
  "created": "2026-09-27T00:00:00Z",
  "last_used": "2026-09-27T00:00:00Z"
}
```

- `otpauth://`는 Google Authenticator Key URI Format 표준(`otpauth://totp/<issuer>:<account>?secret=<BASE32>&issuer=<…>`, HOTP는 `otpauth://hotp/…&counter=N`)이다 ([Key-Uri-Format wiki](https://github.com/google/google-authenticator/wiki/Key-Uri-Format)). 저장 시 secret을 이 URI로 정규화하면 브로커가 `totp-rs` 등으로 30초 코드를 무인 생성할 수 있고, Cloudflare 대시보드 2FA 단계가 무인화된다.
- 1Password/Bitwarden을 백엔드로 쓸 때는 같은 의미론을 각 볼트의 필드(`op://` 필드, Bitwarden login.totp — Bitwarden도 동일 `otpauth://` 포맷 사용)로 매핑.

### 4.3 OxiBrowser 통합 실무

- 의존성: `keyring`(v1 피처)로 시작 → 저장소 교체 가능성이 생기면 `keyring-core` + `apple-native-keyring-store`(`keychain` 백엔드, 데이터 보호 키체인은 번들 요구로 제외)로 이관.
- 노출면: CLI 서브커맨드(`oxibrowser credential get/put/list --agent --site`) + session REPL 명령 + CDP OXI 도메인의 **참조 기반** API(페이지 JS에는 값 노출 금지). 스킬(`skills/`) 문서에 "비밀 직접 출력 금지, 레드랙션" 규칙 명시.
- 온보딩 UX: `oxibrowser credential onboard --site https://dash.cloudflare.com` → keychain ACL 승인 1회 발생(사용자가 로그인 비밀번호 입력) → 이후 무인. partition 문제가 재발하면 안내 문서로 `set-generic-password-partition-list -S 'apple-tool:,apple:,teamid:…'` 온보딩 스크립트 제공.

---

## 실행 가능한 권고

1. **3계층으로 간다**: OxiBrowser 스킬/CLI → 내장 시크릿 브로커 모듈(`keyring-core` + `apple-native-keyring-store`) → login 키체인. 브로커는 백엔드 트레이트(`LocalKeychain | OnePassword | Bitwarden`)로 추상화해 `security` CLI 없이 SecItem 직접 사용.
2. **온보딩은 1회성 사용자 절차로 분리**하고 문서화: 자격증명 공급(Touch ID/키체인 비번 프롬프트 1회) + 에이전트 바이너리 안정 서명 + 필요시 `security set-generic-password-partition-list` 스크립트. 이후 모든 읽기는 무프롬프트.
3. **서비스 키 컨벤션 고정**: `com.oxibrowser.agent/<agent-id>/<site>` + account에 `<login>|kind`. 감사를 위해 `last_used` 갱신, 자격증명 조회 로그(값 아님)를 남긴다.
4. **저장 포맷은 JSON 레코드**, `otpauth://totp/…` URI로 TOTP 정규화(Google Key URI Format). 브로커가 TOTP 코드를 실시간 생성해 2FA 단계 무인화.
5. **무인/감독 분기**: 자율 실행용 자격증명은 전용 볼트/전용 키체인(에이전트별 파일)으로 격리하고, 강권한 작업(Tailscale 콘솔 승인, iCloud 로그아웃류)은 무인화하지 말고 알림→원클릭 승인 흐름으로 설계.
6. **유출 경로부터 막는다**: HAR 캡처·스크린샷·로그·CDP 이벤트에서 secret 값을 레드랙션하는 필터를 브로커 통합 전에 구현. 1Password 문서도 명시하듯 출력 마스킹은 보장이 아니다.
7. **6~12개월 내 파일 기반 키체인의 지위 변동을 감시**: TN3137 기조는 데이터 보호 키체인 이행이지만 CLI/데몬 경로는 당분간 file-based가 유일하므로, 브로커 추상화 뒤에 숨겨 교체 비용을 0으로 둔다.

## 주요 출처

1. TN3137: On Mac keychain APIs and implementations — https://developer.apple.com/documentation/technotes/tn3137-on-mac-keychains
2. Access Control Lists (Keychain) — https://developer.apple.com/documentation/security/access-control-lists
3. ACL Authorization Keys (kSecACLAuthorizationPartitionID) — https://developer.apple.com/documentation/security/acl-authorization-keys
4. SecACLCreateWithSimpleContents (ACL 변경 시 키체인 비밀번호 요구) — https://developer.apple.com/documentation/security/secaclcreatewithsimplecontents(_:_:_:_:_:)
5. Restricting keychain item accessibility (SecAccessControl/Touch ID) — https://developer.apple.com/documentation/Security/restricting-keychain-item-accessibility
6. TN2206: macOS Code Signing In Depth — https://developer.apple.com/library/archive/technotes/tn2206/
7. Apple OSS Security — SecurityTool/security.c (`set-*-partition-list`) — https://github.com/apple-oss-distributions/Security/blob/main/SecurityTool/macOS/security.c
8. `keyring` 4.2.0 (docs.rs) — https://docs.rs/keyring/latest/keyring/
9. `apple-native-keyring-store` (docs.rs) — https://docs.rs/apple-native-keyring-store/latest/apple_native_keyring_store/
10. ASCredentialProviderViewController (AuthenticationServices) — https://developer.apple.com/documentation/authenticationservices/ascredentialproviderviewcontroller
11. Apple Platform Security Guide — https://help.apple.com/pdf/security/en_US/apple-platform-security-guide.pdf
12. kSecAttrSynchronizable — https://developer.apple.com/documentation/security/ksecattrsynchronizable
13. Apple Newsroom — macOS Tahoe 26 발표 — https://www.apple.com/newsroom/2025/06/macos-tahoe-26-makes-the-mac-more-capable-productive-and-intelligent-than-ever/
14. 1Password CLI — Load secrets into scripts — https://developer.1password.com/docs/cli/secrets-scripts
15. 1Password CLI — App integration / security — https://developer.1password.com/docs/cli/app-integration-security 및 https://developer.1password.com/docs/cli/app-integration
16. Bitwarden CLI 문서(`bw login/unlock/serve`) — https://bitwarden.com/help/cli/
17. rbw(비공식 Bitwarden CLI, 에이전트 데몬 모델) — https://github.com/doy/rbw
18. Google Authenticator Key URI Format(otpauth://) — https://github.com/google/google-authenticator/wiki/Key-Uri-Format

부가 확인: `security(1)` man page 본기기(macOS SDK)에서 `set-generic-password-partition-list` 등 3종 명령 및 `-k` 요구 사항 확인. crates.io에서 keyring 4.2.0 / apple-native-keyring-store 1.0.2 / security-framework 3.7.0 버전 확인(2026-09-27).
