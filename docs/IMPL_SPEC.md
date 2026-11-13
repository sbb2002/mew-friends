# Mew Friends — 코드 구현 명세 (MVP)

> 작성 2026-11-13 · Claude. 기획 = `docs/PLAN.md`. 이 문서는 **모듈 계약**이다 — 함수 이름 · 인자 · 반환형은 `src/` 의 뼈대(stub)와 같고, 바꾸지 않는다.
> 대상 = 운영자 1인이 쓰는 Windows 10/11 전용 MVP. Rust 2024, `x86_64-pc-windows-msvc`.

## 0. 공통 규칙 (모든 작업)

- **맡은 파일만 고친다.** 다른 모듈 · `main.rs` · `model.rs` · `util.rs` · `poll.rs` 는 읽기만. 공개 함수 시그니처(이름 · 인자 · 반환형)는 바꾸지 않는다(비공개 함수 · 타입은 자유).
- `Cargo.toml` 의존성은 이미 들어 있다. **새 크레이트를 추가하지 않는다.** `windows` 크레이트 feature 가 모자라면 `[dependencies.windows] features` 목록에 **한 줄 추가만** 허용(보고서에 적을 것).
- `unwrap()` 으로 죽지 않는다 — 실패는 `eprintln!("[모듈] …")` 로 남기고 계속 간다(디버그 빌드에서만 콘솔이 보인다).
- 주석 · 문서 · 커밋 메시지는 **한국어**.
- 마치면 `cargo check --all-targets` 와 `cargo test` 가 통과해야 한다(경고는 허용, 오류 0). 다른 작업의 stub(`todo!()`)은 그대로 두면 컴파일된다.
- **커밋한다**(맡은 파일만, 메시지 예: `feat(diff): 알림 판정 · 저장 구현`). push 하지 않는다.
- 보고서: 바꾼 파일, 지킨/못 지킨 명세 항목, 실측한 것과 못 한 것(「확인 못 함」으로 명시), 추가한 feature.

### 공용 모듈 (이미 구현됨 — 그대로 쓴다)
- `model.rs`: `Preview` · `Item` · `TweetsFile` · `Tweet` 역직렬화, `Preview::order() / name_ko() / channel_url()`, `Item::members() / display_title()`, `Tweet::translated()`, `short_name_ko()`, `DEFAULT_ORDER`, `POLL_SECS`.
- `util.rs`: `wide(&str)->Vec<u16>`, `open_url(&str)`(http(s)만), `app_dir()`(= `%APPDATA%\mew_friends`), `parse_utc(&str)`, `fmt_kst(&str)`(→ `11/13 (금) 22:00`), `APP_NAME`, `AUMID`.
- `poll.rs`: 75초 폴링 스레드 → `PollResult{preview, tweets}` + 메인 스레드에 `WM_APP_POLL`.
- `main.rs`: 메시지 루프 · 스레드 타이머 · 모듈 연결(읽어 두면 각 함수가 언제 불리는지 알 수 있다).

### 실행 구조 (main.rs)
- 단일 스레드 Win32 메시지 루프(메인 스레드)에서 캐릭터 창 · 패널 창 · 트레이가 모두 돈다. 폴링만 별도 스레드.
- 스레드 타이머(hwnd 없음)마다 `Critters::tick()` → 다음 간격 = `frame_interval_ms()`.
- 루프 한 바퀴마다 `Critters::take_clicks()` · `Tray::poll()` 을 부른다.
- 폴링 결과마다 `diff_preview` / `diff_tweets` → 토스트 · `set_live` · `set_unread` → `state::save`.

### 데이터 (참고)
- `fixtures/preview.sample.json` · `fixtures/tweets.sample.json` = mewtype-scheduler 저장소의 시험 데이터.
- 계약 정본 = mewtype-scheduler `docs/SPEC.md` 계약 A · I. `state` ∈ `announced|upcoming|watching|live|end`. 시각은 전부 UTC ISO(`Z`).

---

## 4. 작업 A — `diff.rs` · `state.rs`

### 4-1. `diff_preview(seen, p) -> Vec<Notice>`
1. `seen.preview_ready == false`(첫 실행): `p.items` 중 `state == "live"` 인 id 를 전부 `live_ids` 에 넣고, `preview_ready = true`, **빈 목록 반환**(첫 실행엔 알리지 않는다).
2. 그 뒤: `p.items` 를 순서대로 보며 `state == "live"` 이고 `id` 가 `live_ids` 에 없으면
   - `live_ids` 에 넣고
   - `Notice::LiveStarted { item_id: id, members: item.members(), title: item.display_title(), url }` 를 낸다.
   - `url` = `item.url`(비어 있지 않으면) → 없으면 `p.channel_url(&item.channel_key)` → 그것도 없으면 `"https://www.youtube.com/"`(빈 문자열 금지, 주소를 지어내지 않는다).
3. `assumed_live` 는 보지 않는다(`state` 만 본다).
4. `live_ids` 는 `SEEN_CAP`(2000) 넘으면 앞(오래된 것)부터 버린다.

### 4-2. `diff_tweets(seen, t, now) -> Vec<Notice>`
모든 멤버(`t.tweets` 의 key 를 **정렬한 순서**로 — 결과 순서를 결정적으로)의 모든 글에 대해:
- 만료 판정: `expires_at` 을 `util::parse_utc` 로 읽어 `<= now` 면 만료. 못 읽으면 만료 아님.
1. `seen.tweets_ready == false`(첫 실행): 모든 글 id 를 `tweet_ids` 에 넣고 `pending_tl` 은 비운 채, `tweets_ready = true`, **빈 목록 반환**.
2. 그 뒤, 글마다:
   - 만료된 글: `pending_tl` 에 있으면 빼고, `tweet_ids` 에 없으면 넣는다. 알리지 않는다.
   - `tweet_ids` 에 없는 글(새 글): `tweet_ids` 에 넣는다. `tweet.translated()` 가 있으면 `Notice::NewTweet` 를 내고, 없으면 `pending_tl` 에 넣는다.
   - `pending_tl` 에 있는 글: `translated()` 가 생겼으면 `NewTweet` 를 내고 `pending_tl` 에서 뺀다.
   - `NewTweet { member: <key>, tweet_id, text_ko: <번역>, url }` — `url` = `tweet.url`, 없으면 `https://x.com/i/status/{id}`.
3. `pending_tl` 에 있는데 이번 파일에 없는 id 는 `pending_tl` 에서 뺀다(사라진 글은 번역이 와도 알릴 수 없다).
4. `tweet_ids` 는 `SEEN_CAP` 넘으면 앞부터 버린다.
5. 같은 실행 안에서 같은 글로 두 번 알리지 않는다.

### 4-3. `state.rs`
- 경로 = `util::app_dir().join("state.json")`.
- `load()`: 파일이 없거나 해석 실패면 `AppState::default()`(설정 = 둘 다 켜짐, `Seen` 은 ready=false). 해석 실패면 원본을 `state.json.bad` 로 이름 바꿔 둔다.
- `save()`: `serde_json::to_string_pretty` → 같은 폴더의 `state.json.tmp` 에 쓰고 `std::fs::rename` 으로 바꿔치기(쓰다 꺼져도 깨지지 않게). 실패는 `eprintln!` 만.

### 4-4. 시험 (`#[cfg(test)]`, 네트워크 · 파일 없이)
최소 아래를 시험한다(JSON 문자열을 직접 만들어 `model::parse_*` 로 읽어도 되고, fixture 를 `include_str!("../fixtures/…")` 로 써도 된다):
- 첫 실행은 알림 0건, 그 뒤 같은 데이터 다시 → 0건.
- 새 방송이 live 가 되면 1건, 같은 데이터 다시 → 0건.
- 합동(`collab_with`) → `members` 에 둘 다, 순서 = 주 멤버 먼저.
- `url` 없는 live → 채널 주소로.
- 새 트윗 번역 있음 → 1건. 번역 없음 → 0건, 다음에 번역이 생기면 1건, 그다음 0건.
- 번역 오기 전에 만료 → 끝까지 0건. 번역 없는 채 파일에서 사라짐 → `pending_tl` 비워짐.
- `SEEN_CAP` 넘침 → 길이 유지, 가장 오래된 것 빠짐.
- `Seen` · `AppState` 가 JSON 왕복(직렬화 → 역직렬화)해도 같다. 옛 파일에 필드가 빠져 있어도 읽힌다(`{}` → default).

---

## 6. 작업 B — `toast.rs` · `tray.rs` · `autostart.rs`

### 6-1. `toast.rs` (크레이트 `tauri-winrt-notification` 0.8)
- `init()`:
  1. 알림 앱 ID 등록 — 레지스트리 `HKCU\Software\Classes\AppUserModelId\sbb2002.MewFriends` 에 문자열 값 `DisplayName = "Mew Friends"`, `IconUri = <앱 폴더>\icons\yuno.png`(전체 경로). 이 방식으로 설치 없는 exe 의 토스트에 앱 이름이 붙는다(**실제로 그런지는 Phase 0 확인 항목** — 결과를 보고서에 적을 것).
  2. 멤버 아이콘 5개(`assets/icons/<key>.png`, `include_bytes!`)를 `util::app_dir()/icons/<key>.png` 로 써 둔다(이미 있고 크기가 같으면 건너뜀).
  - 실패해도 패닉하지 않는다.
- `show(title, body, url, member)`:
  - `Toast::new(util::AUMID)` + `.title(title)` + `.text1(body)` + 아이콘(`member` 의 png, `IconCrop::Circular`) + 소리 기본.
  - `url` 이 있으면 `.on_activated(move |_| { util::open_url(&url); Ok(()) })` — 토스트를 누르면 브라우저로 연다. 프로세스는 계속 살아 있으므로 콜백이 돈다.
  - 크레이트 API 가 위 이름과 다르면 그 버전의 실제 API 에 맞춘다(소스: `~/.cargo/registry/src/*/tauri-winrt-notification-0.8.*`). 맞춘 내용을 보고서에.
  - 실패는 `eprintln!`.

### 6-2. `autostart.rs`
- 레지스트리 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 의 값 `MewFriends` = `"<현재 exe 전체 경로>"`(따옴표 포함, `std::env::current_exe()`).
- `is_enabled()`: 값이 있고 경로가 현재 exe 와 같으면(대소문자 무시) true.
- `set_enabled(true)` = 쓰기, `set_enabled(false)` = 값 삭제(없으면 성공 취급).
- `windows` 크레이트의 `RegOpenKeyExW / RegSetValueExW / RegDeleteValueW / RegQueryValueExW` 사용.

### 6-3. `tray.rs` (크레이트 `tray-icon` 0.26, 메뉴 = `tray_icon::menu`)
- 아이콘 = `assets/icons/yuno.png`(`include_bytes!` → `image` 로 RGBA 디코드 → `tray_icon::Icon::from_rgba`). 툴팁 `"Mew Friends"`.
- 메뉴(위에서부터):
  1. 하위 메뉴 「패널 열기」 — `order` 순서로 멤버 5명(`model::short_name_ko`) → `TrayAction::OpenPanel(key)`
  2. 구분선
  3. 체크 「방송 시작 알림」(초기값 `settings.live_alerts`) → `SetLiveAlerts(체크 상태)`
  4. 체크 「새 트윗 알림」(초기값 `settings.tweet_alerts`) → `SetTweetAlerts(…)`
  5. 체크 「Windows 시작 시 실행」(초기값 `autostart`) → `SetAutostart(…)`
  6. 구분선
  7. 「정보」 → 메시지 상자(`MessageBoxW`, 아이콘 정보): `"Mew Friends (MVP)\n비공식 팬 제작 · AI 생성 일러스트 · 공식과 무관\n데이터: mewtype-schduler.vercel.app"` — 이 동작은 `TrayAction` 을 내지 않고 tray 안에서 처리.
  8. 「종료」 → `Quit`
- `poll()`: `MenuEvent::receiver().try_recv()` 를 비울 때까지 돌려 id 로 동작을 만든다. 체크 항목은 이벤트 시점의 `is_checked()` 값을 그대로 쓴다.
- `Tray` 는 `TrayIcon` 과 메뉴 항목 핸들을 들고 있어야 한다(drop 되면 아이콘이 사라진다).
- 왼쪽 클릭으로 메뉴가 뜨게(`with_menu_on_left_click(true)`가 있으면 사용).

---

## 7. 작업 C — `critter.rs`

### 7-1. 창
- 멤버마다 창 1개. 창 클래스 하나(`MewFriendsCritter`), 스타일 `WS_POPUP`, 확장 스타일 `WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`.
- 그리기 = **`UpdateLayeredWindow`**(픽셀 단위 알파). 화면 DC 호환 32비트 top-down DIB(`CreateDIBSection`, BGRA, **premultiplied**)에 그리고 `BLENDFUNCTION{AC_SRC_OVER, 0, 255, AC_SRC_ALPHA}` + `ULW_ALPHA`.
  - `tiny_skia::Pixmap` 은 premultiplied RGBA → 복사할 때 R·B 를 바꿔 BGRA 로.
  - 알파 0 픽셀은 클릭이 아래로 통과한다(layered 창의 기본 동작).
- **창 뒤 바탕화면에만**: 만들 때 `SetWindowPos(HWND_BOTTOM, …, SWP_NOACTIVATE|SWP_NOMOVE|SWP_NOSIZE)`. wndproc 에서
  - `WM_WINDOWPOSCHANGING`: `WINDOWPOS.flags` 에 `SWP_NOZORDER` 가 없으면(= z 순서를 바꾸려는 시도) `hwndInsertAfter = HWND_BOTTOM` 으로 고친다. 캐릭터를 옮길 때 쓰는 `SetWindowPos` 는 `SWP_NOZORDER|SWP_NOACTIVATE` 를 함께 준다.
  - `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`(눌러도 포커스 · 앞으로 오기 없음).
  - `WM_LBUTTONUP` → 그 멤버를 클릭 대기열에 넣는다(눌렀다 뗀 위치가 같은 창일 때). `WM_LBUTTONDOWN` 에서 끌기 시작은 하지 않는다(MVP 는 끌기 없음).
  - `WM_SETCURSOR` → 손 모양 커서(`IDC_HAND`).
- 창 상태는 `GWLP_USERDATA` 에 멤버 index 를 두거나 `thread_local!` 표로 찾는다(메인 스레드 전용).
- 창은 `ShowWindow(SW_SHOWNOACTIVATE)`.

### 7-2. 크기 · 배치
- DPI: `GetDpiForSystem()`(또는 창의 `GetDpiForWindow`) / 96 = `scale`.
- 작업 영역 = `SystemParametersInfoW(SPI_GETWORKAREA)`(주 모니터, 작업표시줄 제외).
- 캐릭터 화면 크기: 높이 약 `130 * scale` px. 창은 캐릭터 위로 LIVE 표시 자리(`28 * scale`)를 더 가진다.
- 발바닥 = 작업 영역 아랫선 −2px.
- 정지 멤버 4명: 작업 영역 폭을 `order` 순서로 5등분한 칸의 가운데에 고정(유노 칸도 비워 둔다 — 유노는 걷는다).
- 화면 해상도 · 작업표시줄이 바뀌면(`WM_DISPLAYCHANGE` · `WM_SETTINGCHANGE` 를 아무 캐릭터 창이 받으면) 다시 배치.

### 7-3. 그림
- **정지 4명**(`arale` · `nonoka` · `ritsu` · `miyako`): `assets/static/<key>.png`(240×280, `include_bytes!` → `image` 디코드). 높이 기준으로 줄여 그린다. (주의: 이 그림은 **임시본**이다 — 최종 부위 그림 확정 전.)
- **유노**: `assets/yuno/*.webp` 10장을 아래 리그로 조립한다. 원본 = mewtype-scheduler `src/frontend/js/bdayscene-yuno.js` 137~182행(그 파일을 읽을 수 있으면 읽어서 확인). 리그 좌표계(약 240×234, 발 ≈ y 229):
  - `drawRot(img, x, y, w, h, deg, ox, oy)` = (ox,oy) 를 중심으로 deg 회전한 뒤 이미지를 (x,y,w,h) 에 그린다.
  - 그리는 순서:
    1. `tail`: `drawRot(tail, 157, 159, 82, 70, q.tail, 157+0.15*82, 159+0.75*70)`
    2. `body`: (120,170) 중심으로 `q.bodyRoll` 회전, 다각형 클립(x=59,y=154,w=126,h=80, 점 (u,v) = [(.25,0),(.75,0),(.75,.28),(.78,.66),(1,.85),(1,1),(0,1),(0,.85),(.22,.66),(.25,.28)] → (x+u*w, y+v*h)) 안에 `body` 를 (59,154,126,80) 으로.
    3. `arm_l`: `drawRot(arm_l, 50, 157, 43, 53, q.left, 86, 164)`
    4. `arm_r`: `drawRot(arm_r, 114, 157, 45, 52, q.right, 153, 164)`
    5. 머리 묶음: (32+87.5, 10+0.88*156) 중심으로 `q.headAngle` 회전, `headY` 만큼 아래로, 원점 (32,10) 으로 옮긴 뒤
       - `head_open`(눈 뜸) 또는 `head_closed`(깜빡임) 를 (0,0,175,156)
       - `drawRot(ear_l, 0,0,175,156, -q.ear, 0.28*175, 0.45*156)`
       - `drawRot(ear_r, 0,0,175,156,  q.ear, 0.72*175, 0.45*156)`
       - `drawRot(headset, 142, 97, 42, 40, q.mic, 142+21, 97+20)`
  - 전체 리그에 `bounce`(위아래, 리그 단위 px)를 더한다. 왼쪽으로 걸을 때는 좌우 반전(리그 x 중심 120 기준).
  - 이미지는 시작 때 한 번 디코드해 `Pixmap` 으로 들고 있는다(매 프레임 디코드 금지).

### 7-4. 유노 움직임 (걷기 — 새로 만든 것, 다리 부위 없음)
- 상태 기계: `Walk`(6~15초) ↔ `Idle`(4~10초) 무작위 반복. 무작위 = 시각 기반 간단 LCG(새 크레이트 금지).
- `Walk`: 속도 `40 * scale` px/초, 작업 영역 양 끝에 닿으면 방향 반대. 걸음 주기 0.5초:
  `bounce = -abs(sin(π·t/0.5)) * 4`, `bodyRoll = sin(2π·t/1.0) * 3°`, `left = sin(2π·t/1.0) * 12°`, `right = -left`, `ear = sin(2π·t/1.0) * 6°`, `tail = sin(2π·t/0.7) * 10°`, `headAngle = sin(2π·t/1.0) * 2°`, `headY = 0`, `mic = 0`.
- `Idle`: 제자리, 숨쉬기 `bounce = sin(2π·t/3.2) * 1.5`, `ear = sin(2π·t/4.0) * 3°`, `tail = sin(2π·t/2.6) * 6°`, 나머지 0.
- 깜빡임(두 상태 공통): `pct = ((t+0.7) mod 7.6) / 7.6 * 100`, `44.5 <= pct < 46.3` 이면 `head_closed`(웹 원본과 같은 규칙).
- 시간 = `Instant` 경과(프레임 수가 아니라 실제 시간 기준).

### 7-5. LIVE · 새 트윗 표시
- `set_live(member, true)` → 캐릭터 머리 위 가운데에 `assets/live_badge.png`(56×22, scale 배) + 캐릭터 전체가 2초 주기로 아주 약하게 위아래(정지 멤버도 이때만 움직인다: `bounce = sin(2π·t/2) * 2`).
- `set_unread(member, true)` → 캐릭터 오른쪽 위에 지름 `10*scale` 의 분홍(#ff4f8b) 원 + 흰 테두리 1.5px.
- 값이 바뀔 때만 다시 그린다(정지 멤버).

### 7-6. 속도 · 자원
- `frame_interval_ms()`: 유노 `Walk` = 50, `Idle` 또는 LIVE 표시 중 = 100, 그 밖 = 250(깜빡임을 놓치지 않을 정도로 — 깜빡임 길이 ≈ 137ms 이므로 Idle 는 100).
- **전체 화면 앱 · 프레젠테이션 중엔 멈춤**: `SHQueryUserNotificationState` 가 `QUNS_BUSY` · `QUNS_RUNNING_D3D_FULL_SCREEN` · `QUNS_PRESENTATION_MODE` 면 그리지 않고 `frame_interval_ms()` = 1000. (1초마다 확인)
- 정지 멤버는 바뀔 때만 다시 그린다. 유노는 프레임마다.

### 7-7. 시험
- 리그 · 상태 계산(포즈 값, 깜빡임 판정, Walk/Idle 전환, 벽에서 방향 반전)은 Win32 없이 부를 수 있는 순수 함수로 나누고 `#[cfg(test)]` 로 시험한다.
- 가능하면 디버그 실행으로 창이 실제로 뜨는지 확인하고, 유노 한 프레임을 PNG 로 저장하는 시험(`#[test] #[ignore]` + `cargo test -- --ignored`)을 둬 조립 결과를 눈으로 확인할 수 있게 한다. 저장 위치 = `target/critter_frame_*.png`.

---

## 8. 작업 D — `panel.rs`

### 8-1. `build_data(member, preview, tweets, now) -> PanelData` (순수)
- `name_ko` = `preview.name_ko(member)`; 데이터가 비어 key 가 그대로 오면 `model::short_name_ko(member)`.
- `live` = `preview.items` 중 `state == "live"` 이고 `members()` 에 `member` 가 있는 **첫** 항목 → `(display_title(), url)`. `url` 규칙은 작업 A 4-1 과 같다(item.url → channel_url → `https://www.youtube.com/`).
- `tweets` = `tweets.tweets[member]` 중 **만료 안 된 글**(`expires_at` > now, 못 읽으면 보임)만, **최신이 앞**(파일은 최신이 뒤 → 뒤집기). 최대 50건(웹 `MAX_THREAD` 와 같음).
  - `text` = `translated()` 또는 `"번역 준비 중…"`(웹과 같은 문구).
  - `when` = `posted_at` → 없으면 `received_at` 를 `util::fmt_kst` 로, 둘 다 없으면 `""`.
  - `url` = `tweet.url` 또는 `https://x.com/i/status/{id}`.

### 8-2. `show(data)` — Win32 창 (메인 스레드)
- 패널은 **하나만**. 이미 떠 있으면 내용을 바꾸고 `ShowWindow(SW_SHOW)` + `SetForegroundWindow`.
- 창: 일반 창(`WS_OVERLAPPED|WS_CAPTION|WS_SYSMENU|WS_THICKFRAME`, 최소화 · 최대화 버튼 없음), 제목 = `"{name_ko} — Mew Friends"`, 크기 `360×480`(× DPI scale), 처음엔 작업 영역 오른쪽 아래 구석(여백 16px). 크기 조절 시 안의 컨트롤을 다시 배치.
- 글꼴: `CreateFontW` `"Malgun Gothic"`, 높이 `-15 * scale`, 모든 자식 컨트롤에 `WM_SETFONT`.
- 위에서부터:
  1. 방송 중이면 버튼 「▶ 방송 보러 가기 — {제목}」(긴 제목은 버튼 폭에서 잘림 허용) → `util::open_url`. 방송 중 아니면 이 자리 없음.
  2. 트윗 목록: **owner-draw 가변 높이 LISTBOX**(`LBS_OWNERDRAWVARIABLE | LBS_NOINTEGRALHEIGHT | LBS_NOTIFY | WS_VSCROLL`).
     - `WM_MEASUREITEM`: `DrawTextW(DT_CALCRECT|DT_WORDBREAK)` 로 본문 높이 + 시각 한 줄 + 위아래 여백 8px.
     - `WM_DRAWITEM`: 시각(회색 #888, 작게) 한 줄 위, 그 아래 본문(줄바꿈). 선택 · 마우스 강조는 연한 배경. 「번역 준비 중…」 은 회색.
     - 항목 클릭(`LBN_SELCHANGE`) → 그 글의 `url` 을 `util::open_url`.
     - 글이 없으면 목록 대신 가운데 정적 글 「표시할 트윗이 없어요」.
  3. 맨 아래 작은 회색 글: 「비공식 팬 제작 · 공식과 무관」.
- 닫기(X) = 창 파괴(`DestroyWindow`), 다음 `show` 때 새로 만든다. `Esc` 키로도 닫힌다(목록 컨트롤을 서브클래싱하거나 `WM_KEYDOWN` 처리 — 방법은 자유).
- 패널은 항상 위가 아니다. 다른 창처럼 행동한다.
- 데이터는 창 상태(thread_local 또는 `GWLP_USERDATA`)에 둔다.

### 8-3. 시험
- `build_data` 를 `#[cfg(test)]` 로: 만료 제외 · 최신이 앞 · 번역 없음 문구 · 시각 우선순위 · live 주소 대체 · 합동 방송(`collab_with` 에 member) · 데이터 비었을 때 이름.
- 가능하면 디버그 실행으로 창이 뜨는지 확인(확인 못 했으면 보고서에 명시).
