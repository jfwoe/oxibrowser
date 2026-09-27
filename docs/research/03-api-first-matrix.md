# 03. API-퍼스트 무인 경로 매트릭스

작성일: 2026-09-27 · 대상: OxiBrowser 기반 서버 세팅 에이전트의 "사용자 손빌림" 제거

## 1. 문제 정의

실제 세션에서 에이전트가 브라우저/GUI 로그인 또는 사용자 클릭 없이 진행하지 못한 4개 지점:

| # | 막힌 작업 | 당시 경로 | 필요했던 사용자 개입 |
|---|---|---|---|
| A | Cloudflare 터널 생성·라우팅·커넥터 인증 | `cloudflared tunnel login` → 브라우저에서 zone 선택 | 브라우저 로그인 1회 |
| B | Tailscale 서브넷 라우터 라우트 승인 | 관리콘솔(Machines → route approve) 클릭 | 웹콘솔 로그인 + 클릭 |
| C | macOS iCloud(Apple Account) 로그아웃 | System Settings → Apple ID → Sign Out | GUI 조작 + 암호 |
| D | 시스템설정 GUI 승인(시스템 확장/네트워크 확장 등) | System Settings 승인 대화상자 | 관리자 암호 입력 |

결론부터: **A, B는 브라우저·콘솔 없이 100% API로 무인화 가능**하다(사전 1회 토큰 발급 필요). **C는 Apple이 무인 API를 제공하지 않아 구조적으로 불가**, D는 MDM 사전 프로비저닝 또는 코드서명된 헬퍼로만 무인화 가능하고 비관리 Mac이면 사용자 1회 승인이 필수다.

## 2. 요약 매트릭스

| 작업 | 무인 경로 | 필요 자격증명(사전 1회 발급) | 갱신 주기 | 무인화 가능성 |
|---|---|---|---|---|
| CF 터널 생성 | `POST /accounts/{id}/cfd_tunnel` (config_src=cloudflare) | CF API 토큰 (Account: Tunnel Edit) | 토큰 expires_on 설정 가능, API로 재발급 | **완전 가능** |
| CF 인그레스 라우팅 | `PUT /accounts/{id}/cfd_tunnel/{tun}/configurations` | 〃 | 〃 | **완전 가능** |
| CF 커넥터 인증 | `GET …/cfd_tunnel/{tun}/token` → `cloudflared tunnel run --token` | 터널 토큰(토큰에서 파생) | 터널 단위, 사실상 무기한 | **완전 가능** |
| CF DNS CNAME | `POST /zones/{zone}/dns_records` | CF API 토큰 (+Zone: DNS Edit) | 〃 | **완전 가능** |
| TS 기기 등록 | OAuth 클라이언트 → auth key → `tailscale up --auth-key` | OAuth client ID/secret | 액세스 토큰 1시간 자동 갱신 | **완전 가능** |
| TS 라우트 승인 | `autoApprovers.routes` 정책(자동) 또는 `POST /device/{id}/routes` | policy_file 스코프 토큰 | 정책은 장기, 토큰 스코프 관리 | **완전 가능** |
| TS 기기 승인 | `POST /device/{id}/authorized` (webhook `nodeNeedsApproval` 트리거) | devices:core 스코프 | 〃 | **완전 가능** |
| iCloud 로그아웃 | **존재하지 않음** (MDM 명령 목록에 없음) | — | — | **불가** (사용자 개입 or EraseDevice) |
| 시스템설정 GUI 승인 | MDM 프로파일(SystemExtensions, PPPC) 사전 승인 | ADE/MDM 등록 | 프로파일 1회 배포 | 조건부 가능(관리 기기만) |

## 3. Cloudflare — 터널 전 과정 API 토큰화

### 3.1 핵심 사실

- `cloudflared tunnel login`(cert.pem 발급용 브라우저 인증)은 **로컬 관리 터널**에만 필요하다. API로 생성하는 **원격 관리 터널**(`config_src: "cloudflare"`)은 브라우저 인증이 전혀 없다. 출처: Cloudflare "Create a tunnel (API)" 가이드
- 자격증명이 2개로 분리된다: ① 프로비저닝용 **CF API 토큰**(에이전트/시크릿 저장소 보관) ② 커넥터용 **터널 토큰**(cloudflared 호스트에만 전달). 터널 토큰 하나만 있으면 누구든 커넥터를 실행할 수 있으므로 비밀 취급. 출처: Tunnel tokens 문서
- DNS CNAME(`<tunnel_id>.cfargotunnel.com`, proxied=true)은 터널 생성과 별개로 만들어야 한다.

### 3.2 무인 시퀀스

```
1) POST /accounts/{account_id}/cfd_tunnel            {"name":…, "config_src":"cloudflare"}  → tunnel_id
2) PUT  /accounts/{account_id}/cfd_tunnel/{id}/configurations   (ingress 규칙, catch-all 필수)
3) GET  /accounts/{account_id}/cfd_tunnel/{id}/token            → tunnel_token
4) POST /zones/{zone_id}/dns_records    CNAME → {tunnel_id}.cfargotunnel.com, proxied
5) 대상 호스트: cloudflared tunnel run --token $TUNNEL_TOKEN  (또는 service install / docker)
```

### 3.3 토큰 스코프 설계

| 목적 | 스코프 | 범위 제한 |
|---|---|---|
| 터널 생성·설정·토큰 조회 | Account → **Cloudflare Tunnel: Edit** (API 문서명 Connectors Write) | 계정 리소스 한정 |
| DNS 레코드 생성 | Zone → **DNS: Edit** | 해당 zone만 |
| zone 메타데이터 조회 | Zone → **Zone: Read** | 해당 zone만 |
| 토큰 자동 발급/회전(bootstrap) | User → **API Tokens: Write** 또는 Account → Account API Tokens: Write | TTL + 클라이언트 IP 필터 강제 권고 |

설계 원칙(공식 문서 기반):
- **runtime 토큰과 bootstrap(토큰 발급용) 토큰을 분리**한다. `API Tokens Write`는 사실상 "토큰 찍는 기계"이므로 만료일·IP 제한을 걸고 상시 사용 금지. Cloudflare는 토큰-발급 권한에 TTL/IP 제한을 명시적으로 권장. 출처: "Create tokens via API"
- 토큰 값은 생성 응답에 1회만 나오므로 즉시 시크릿 저장소로. Terraform `cloudflare_api_token` 리소스는 값을 state에 평문으로 남기므로 프로덕션 회전용으로는 부적합(공식 문서도 단점 명시). 선언적 관리가 필요하면 `cloudflare_tunnel` 리소스 + 런타임 토큰 주입 방식. 출처: Cloudflare Terraform 가이드
- 권한 그룹 ID는 `GET /user/tokens/permission_groups`에서 조회해 쓰고 하드코딩하지 않는다(이름은 표시용, ID가 실제 키).

### 3.4 실패 모드

- config_src를 `cloudflare`로 안 하고 로컬 관리로 만들면 다시 cert.pem(브라우저 로그인)이 필요해진다 — 원격 관리로 생성할 것.
- ingress 마지막 catch-all 규칙(예: `http_status:404`) 누락 시 configurations PUT이 거부된다.

## 4. Tailscale — 관리콘솔 클릭 전부 API/정책으로 대체

### 4.1 자격증명 체계(3층)

| 자격증명 | 용도 | 수명 |
|---|---|---|
| OAuth client (ID/secret) | API 호출용 액세스 토큰 발급, auth key 발급 | 시크릿은 폐기 전까지 유효(장기) |
| OAuth 액세스 토큰 | 실제 API 호출 | **1시간** (자동 재발급) |
| auth key (`tskey-auth-…`) | 기기 등록 | **최대 90일**, 1~90일 설정 |
| API 키 (`tskey-api-…`) | 개인 계정 API 접근 | 최대 90일 — OAuth 클라이언트로 대체 권장 |

핵심 패턴: **장기 보관하는 것은 OAuth client 시크릿 하나뿐**, auth key는 필요 시점에 API로 발급해 쓰고 버린다. `auth_keys` 스코프 + 허용 태그를 OAuth 클라이언트에 지정하고, 발급은 `POST /api/v2/tailnet/-/keys`(또는 `get-authkey` 유틸리티, `-reusable -preauth -ephemeral` 옵션). 출처: OAuth clients 문서

### 4.2 라우트 자동승인 (관리콘솔 클릭 제거)

정책 파일에 `autoApprovers`를 넣으면 서브넷 라우터의 광고가 자동 승인된다:

```json
{
  "tagOwners":   { "tag:subnet-router": ["autogroup:admin"] },
  "autoApprovers": {
    "routes":    { "192.168.10.0/24": ["tag:subnet-router"] },
    "exitNode":  [ "tag:subnet-router" ]
  },
  "acls": [ { "action": "accept", "src": ["autogroup:member"], "dst": ["192.168.10.0/24:*"] } ]
}
```

- 프로비저닝: `tailscale up --auth-key=$KEY --advertise-tags=tag:subnet-router` → `tailscale set --advertise-routes=192.168.10.0/24`. 태그가 정책과 일치하면 콘솔 개입 없이 즉시 승인. 승인된 범위의 **더 좁은 서브넷** 광고도 허용. 출처: policy file syntax 문서
- **승인 ≠ 트래픽 허용**: `autoApprovers`는 광고 승인만 하고, 실제 통신은 ACL/grant가 허용해야 한다(둘 다 필요). 새 정책 권장은 grants语法(ACL은 유지보수 모드).
- 정책 변경은 이미 광고된 라우트에 소급 적용되지 않는다 — 라우트를 끊고 다시 광고해야 한다(주요 함정).
- API로 승인하는 대안(정책 변경이 곤란할 때): `GET/POST /api/v2/device/{id}/routes` — POST는 enabled 목록 전체를 교체하므로 기존 값을 merge할 것. 스코프는 `devices:routes`(레거시 `routes`). 출처: trust credentials 문서

### 4.3 기기 승인·정책 배포도 API

- 기기 승인: `POST /api/v2/device/{deviceID}/authorized` `{"authorized":true}` — `nodeNeedsApproval` 웹훅에 연결하면 승인 클릭이 완전 자동화된다. 스코프 `devices:core`. 출처: device approval 문서
- 정책 배포: `POST /api/v2/tailnet/-/acl` (사전 `…/acl/validate` 가능). 쓰기엔 `policy_file` 스코프, 읽기는 `policy_file:read`. 출처: trust credentials 문서
- macOS 클라이언트의 시스템 확장 승인(§5.4 참조)은 App Store 버전을 쓰면 시스템 확장 자체가 없어 사전 승인 부담이 사라진다(standalone 빌드만 확장 사용). 출처: Tailscale macOS MDM 문서

## 5. Apple/iCloud — 무인화 가능 경로와 경계

### 5.1 핵심 사실: Apple Account 로그아웃 무인 API는 없다

- Apple MDM 명령 카탈로그(Commands and Queries)에는 **Log Out User, Delete User, Erase Device, Activation Lock 조작** 등이 있으나 "Sign Out Apple Account"는 없다. `Log Out User`는 macOS 세션 로그아웃일 뿐 iCloud 로그아웃이 아니다. 출처: Apple Developer — Commands and queries
- `defaults delete MobileMeAccounts`는 미문서화된 구현 디테일이고, root 컨텍스트에서 실행하면 사용자가 아닌 root의 plist를 건드린다. 키체인 토큰·Find My/Activation Lock 상태를 남겨 불일치 상태를 만들 수 있어 프로덕션 워크플로로 부적합. 최신 macOS에서는 `MobileMeAccounts.plist`가 신뢰 가능한 상태 소스가 아니라는 보고도 있다(작성자 워크스테이션인 macOS 27에서 제거되었다는 보고 포함). 출처: r/macsysadmin 스레드
- 결론: **깨끗한 iCloud 로그아웃은 사용자 상호작용 1회가 필요한 경계**다. 에이전트 설계상 이 단계만 "사용자 확인 게이트"로 명시적 분리하는 것이 정직한 구조다.

### 5.2 대체 무인 경로(목적에 따라)

| 목적 | 무인 경로 | 근거 |
|---|---|---|
| Mac 재할당/반납 | MDM `EraseDevice` → ADE 재등록 (Activation Lock bypass 코드 에스크로 선행) | Commands and queries |
| 퇴사자 계정 제거 | MDM `Delete User` (Activation Lock은 별도 처리) | 〃 |
| 향후 계정 변경 차단 | Restrictions payload `allowAccountModification=false` (macOS 14+) — 로그인된 계정을 로그아웃하지는 않음 | Restrictions 문서 |
| iCloud 없는 무인 설치 | Platform SSO로 IdP(Entra/Okta) 신원 연동 — Apple Account와 무관한 기기 신원 확보 | Platform SSO(UDE) 문서 |

### 5.3 System Settings GUI 조작의 한계 (macOS 13+)

- Ventura에서 System Preferences→System Settings로 재설계되며 직접 AppleScript(`reveal pane`)가 초기 버전에서 깨졌고 13.3에서 부분 복구. 구 pane ID(`com.apple.preference.*`)는 신규 확장 ID(`com.apple.Sound-Settings.extension`)로 교체 필요, `open "x-apple.systempreferences:…"` URL이 가장 안정적. 출처: LateNightSwift 포럼, Apple MDM 문서
- System Events UI 스크립팅은 접근성 계층이 크게 바뀌어 인덱스 기반 경로가 취약하고, 로케일·섹션 유무(Apple ID 섹션 존재 여부 등)에 따라 실패한다. **System Settings 자동화는 탐색(해당 페이지 열기)까지만 쓰고, 상태 변경은 CLI/defaults/프로파일/API로 하는 것**이 원칙. iCloud 로그아웃 버튼 클릭 자동화는 암호 프롬프트·Find My 확인을 만나므로 실질적으로 불가.

### 5.4 "시스템설정 GUI 승인"(시스템 확장 등)의 사전 승인

Tailscale standalone/WARP 설치 시 나오는 시스템 확장 승인은 **MDM SystemExtensions payload로 사전에 무인화** 가능하다:

- payload: `com.apple.system-extension-policy`, device 채널 전용·user-approved MDM 필요.
- Tailscale standalone: Team ID `W5364U7YZB`, 확장 `io.tailscale.ipn.macsys.network-extension`. `AllowedTeamIdentifiers` 또는 더 좁은 `AllowedSystemExtensions`/`AllowedSystemExtensionTypes: NetworkExtension` (동일 Team ID를 두 키에 중복 지정 불가). 출처: Tailscale macOS MDM, Apple SystemExtensions 문서
- Cloudflare WARP도 동일 패턴(배포 순서: 인증서 → 확장 승인 프로파일 → 설정 프로파일 → pkg). Team ID는 설치 패키지에서 `codesign -dv`로 추출, 문서상 고정값 없음. 출처: Cloudflare WARP Intune 배포 문서
- 비관리(개인) Mac이면 이 경로가 없다. 그 경우에도 `authorizationdb` 변경으로 승인 프롬프트를 우회하는 방식은 미지원이며 위험 — 코드서명된 privileged helper(SMAppService/SMJobBless)로 "설치 시 1회만" 인증을 몰아주는 것이 Apple의 지원 모델이다. TCC(접근성·Apple Events 등) 사전 허용도 PPPC 페이로드로 가능하나 사실상 관리 등록이 전제. 출처: Apple 배포 가이드(PPPC), SMJobBless 문서

## 6. 일반 결정트리 — "브라우저 로그인 전에 API 경로부터"

### 6.1 판단 절차 (에이전트가 매 작업마다 통과)

```
Q1. 공급업체 문서화된 API가 있고, 필요 조작이 리소스 CRUD/상태 변경으로 표현되는가?
    → YES: API 경로. "승인 버튼"은 대부분 상태 변경 API(예: TS device/authorized, routes)로 존재한다.
Q2. API 자격증명을 대시보드에서 1회 발급해 위임 가능한가? (스코프 최소화 가능?)
    → YES: 사전 프로비저닝(§6.2) 후 무인 진행.
Q3. 웹 세션/쿠키로만 되는가? (Apple ID, 은행, 웹 전용 관리콘솔, 인간 챌린지)
    → 브라우저 경로: OxiBrowser storage_state/HAR 재사용 + 유효기간 감지. 단 자격증명 재사용은 별도 보안 문서 범위.
Q4. OS GUI 승인인가? (TCC, 시스템 확장, 관리자 인증)
    → MDM 프로파일 사전 승인(관리 기기) 또는 코드서명 helper로 1회 인증으로 압축.
       불가능하면 사용자 확인 게이트로 명시 분리 — 조용히 실패하지 않게 함.
```

우선순위 산식: **반복 빈도 × 개입 비용**. 오늘 막힌 4개 중 A·B는 반복 발생 + 개입 저렴(토큰 1회)이라 최우선 투자, C·D는 빈도가 낮으면 게이트로 남기는 게 맞다.

### 6.2 사전 프로비저닝 패턴

**시크릿 저장·주입 (부팅 시)**
- SOPS+age로 암호화한 시크릿 파일을 저장소에 두고, LaunchDaemon이 부팅 시 복호화해 env/파일로 주입 (파일 0600, 전용 서비스 계정 소유). SOPS는 CNCF 샌드박스, age/KMS/PGP 키 지원. 출처: github.com/getsops/sops
- 대안: 키체인 저장 항목(`security find-generic-password -w`)을 실행 시점에 읽기 — 에이전트가 사용자 세션 밖에서 돌면 ACL 사전 승인 문제가 있어 관리 기기·사용자 에이전트에 적합(별도 연구 주제).
- 원칙: 시크릿은 **런타임에서만 존재**(로그·프로세스 목록·쉘 히스토리 제외), 프로비저닝 토큰과 커넥터 토큰 분리 보관.

**스코프 최소화 체크리스트**
- CF: 리소스를 계정/zone 단위로 한정, 권한 그룹은 필요한 것만(Tunnel Edit + DNS Edit + Zone Read), TTL(`expires_on`)과 클라이언트 IP 필터 설정.
- TS: OAuth 클라이언트에 필요 스코프만(`auth_keys`, `devices:core`, `devices:routes`, `policy_file` 중 최소), 허용 태그 지정 — 태그 소유자가 아니면 키 발급이 거부되는 구조 자체가 권한 경계.
- Apple: 무인 API가 없으므로 "스코프" 개념 대신 Restrictions payload로 변경 가능 범위를 사전에 잠그는 방향.

**갱신 설계**
- CF: runtime 토큰에 만료일 → bootstrap 토큰(API Tokens Write, 짧은 TTL+IP 제한)이 만료 전 재발급·교체·폐기. "새 토큰 배포 → 검증 → 구 토큰 폐기" 순서.
- TS: 보관 장기 시크릿은 OAuth client뿐. 액세스 토큰 1시간마다 재발급(자동). auth key는 90일 상한이 있으므로 **장기 재사용 키를 보관하지 않고** 프로비저닝 시점에 발급(1회용/ephemeral 옵션) — 상한 자체가 "키 재사용 금지" 설계 유도. OAuth 클라이언트 회전은 in-place가 없어 "신규 클라이언트 먼저, 구 클라이언트 폐기 나중".
- 공통: 폐기 API가 있으므로 이탈/재시도 시 즉시 revoke. 토큰 누출 반경 = 스코프 × 수명.

## 7. 실행 가능한 권고

1. **CF API 토큰 1회 발급** (Account: Tunnel Edit + 해당 zone DNS Edit/Zone Read, `expires_on`·IP 필터 설정) → 시크릿 저장소(SOPS+age 또는 키체인)에 보관. 이후 터널·ingress·DNS·터널 토큰 회수까지 §3.2 시퀀스를 스크립트/스킬로 고정. `cloudflared tunnel login` 경로는 폐기.
2. **TS OAuth 클라이언트 1회 생성** (`auth_keys` + `devices:core` + `policy_file`, 태그 `tag:subnet-router` 허용) 후 §4.2 정책(tagOwners + autoApprovers + grant)을 `POST /tailnet/-/acl`로 배포하고 `/acl/validate`로 선행 검증. 관리콘솔의 route/device 승인 클릭을 전부 제거. auth key는 세션마다 발급-폐기.
3. **회전 자동화**: CF는 bootstrap 토큰 기반 재발급 잡(만료 2주 전), TS는 OAuth 액세스 토큰 캐시(1시간) + auth key 즉시 폐기. 양사 모두 "새 것 배포 → 검증 → 옛것 폐기".
4. **Apple 경계를 게이트로 명시**: iCloud 로그아웃·Activation Lock은 무인 API가 없음을 전제로, 에이전트 태스크 그래프에서 `requires_user_confirmation` 노드로 분리. 우회 시도(defaults/MobileMeAccounts, authorizationdb 변경)는 금지 목록에 등재 — 불완전 상태·미지원 경로.
5. **macOS 반복 배포라면 MDM 사전 승인 세트 구축**: SystemExtensions payload(Tailscale `W5364U7YZB` 등) + PPPC + Restrictions(`allowAccountModification=false`). 개인 Mac이면 Tailscale App Store 버전(확장 없음)으로 승인 부담 자체를 제거.
6. **OxiBrowser에 API-퍼스트 게이트 탑재**: 스킬/executor 레이어에서 "브라우저 로그인 시도 직전"에 §6.1 Q1/Q2 검사를 삽입 — 스코프 토큰이 있으면 curl/CLI 경로(`oxibrowser` `session/executor.rs`의 커맨드 실행 경로 활용)로 우회하고, 브라우저는 Q3(웹 전용 신원)에만 사용. HAR/storage_state는 그 다음 방어선.
7. **감사**: CF 토큰 목록/사용 조회, TS 키·디바이스 목록 API를 주간 점검해 미사용 자격증명을 폐기.

## 주요 출처

1. Cloudflare API — Create tunnel(cfd_tunnel): https://developers.cloudflare.com/api/resources/zero_trust/subresources/tunnels/subresources/cloudflared/methods/create/
2. Cloudflare One — Create a tunnel (API): https://developers.cloudflare.com/cloudflare-one/networks/connectors/cloudflare-tunnel/get-started/create-remote-tunnel-api/
3. Cloudflare — Tunnel tokens: https://developers.cloudflare.com/tunnel/reference/tunnel-tokens/
4. Cloudflare — Create tokens via API(스코프/IP/TTL): https://developers.cloudflare.com/fundamentals/api/how-to/create-via-api/
5. Cloudflare — Tunnel with Terraform: https://developers.cloudflare.com/tunnel/guides/terraform/
6. Terraform cloudflare_tunnel 리소스: https://registry.terraform.io/providers/cloudflare/cloudflare/latest/docs/resources/tunnel
7. Tailscale — OAuth clients: https://tailscale.com/docs/features/oauth-clients
8. Tailscale — Policy file syntax(autoApprovers/tagOwners): https://tailscale.com/docs/reference/syntax/policy-file
9. Tailscale — Device approval(POST /device/{id}/authorized, webhook): https://tailscale.com/docs/features/access-control/device-management/device-approval
10. Tailscale — Trust credentials(routes/ACL 스코프, 수명·회전): https://tailscale.com/docs/reference/trust-credentials
11. Tailscale — Auth keys(90일 상한): https://tailscale.com/docs/features/access-control/auth-keys
12. Tailscale — Subnet routers: https://tailscale.com/docs/features/subnet-routers
13. Tailscale — macOS MDM(Team ID W5364U7YZB, 확장 ID): https://tailscale.com/kb/1286/macos-mdm
14. Apple — MDM Commands and queries(로그아웃 명령 부재): https://developer.apple.com/documentation/devicemanagement/commands-and-queries
15. Apple — SystemExtensions payload: https://developer.apple.com/documentation/devicemanagement/systemextensions
16. Apple — 배포 가이드 PPPC: https://support.apple.com/guide/deployment/dep38df53c2a/web
17. LateNightSwift — Ventura System Settings AppleScript 호환성: https://forum.latenightsw.com/t/applescripts-not-working-on-new-system-settings-on-macos-ventura/3986
18. r/macsysadmin — MobileMeAccounts.plist 제거 보고(macOS 27): https://www.reddit.com/r/macsysadmin/comments/1wiyhfa/mobilemeaccountsplist_rmd_from_macos_27/
19. SOPS(암호화 시크릿 관리): https://github.com/getsops/sops
