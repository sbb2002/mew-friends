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
  4-1. 체크 「새 트윗 말풍선 자동으로 띄우기」(초기값 `settings.auto_bubble`, 기본 켬) → `SetAutoBubble(…)`
  4-2. 구분선
  4-3. 체크 「캐릭터 · 말풍선 항상 위에」(초기값 `settings.always_on_top`, 기본 끔 = 다른 창 뒤에서 방해하지 않기) → `SetAlwaysOnTop(…)`
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

## 8. 작업 D — `panel.rs` · `panel/{data,sheet,card,view}.rs` · `net.rs` · `player.rs` (트윗 말풍선)

> 2026-11-13 개정: 목록 창(LISTBOX) → **캐릭터 바로 위 말풍선**. 생김새 · 구성 = mewtype-scheduler 트윗 시트(`js/tweets.js` · `css/tweets.css` `.tws`)를 그대로 옮긴다.

### 8-1. `build_data(member, preview, tweets, now) -> PanelData` (순수)
- `name_ko` = `preview.name_ko(member)`; 데이터가 비어 key 가 그대로 오면 `model::short_name_ko(member)`.
- `live` = `preview.items` 중 `state == "live"` 이고 `members()` 에 `member` 가 있는 **첫** 항목 → `(display_title(), url)`. `url` 규칙은 작업 A 4-1 과 같다(item.url → channel_url → `https://www.youtube.com/`).
- 글 = `tweets.tweets[member]` 중 **만료 안 된 글**(`expires_at` > now, 못 읽으면 보임)의 최신 50건(웹 `MAX_THREAD`).
  - `text` = `translated()` 또는 `"번역 준비 중…"`. `orig` = 원문 `text`. `url` = `tweet.url` 또는 `https://x.com/i/status/{id}`. `handle` = 글의 `handle` → 채널 `handle`.
  - `media` = 첨부 그림 주소(영상 · GIF = 썸네일), `quote` = 인용(원문 또는 번역이 있을 때만). `x_name` = 채널 원어 이름(원문 면 — fxtwitter 를 받기 전).
  - 시각(tweetlogic `whenIso`) = `posted_at` → Snowflake id 에서 계산 → 없으면 `received_at`(「HH:MM 등록」).
- `units` = 글타래 묶기(tweetlogic `chains`): `reply_to_id` 가 목록 안 글이면 뿌리까지 한 덩어리, 덩어리 안 = 시각순, 덩어리 자리 = 가장 늦은 글 시각(오래된 덩어리가 위). 덩어리마다 날짜 구분선 문구 `「11월 13일 (금)」`.
- `chips` = `preview.order()` 멤버마다 (트윗이 있으면 누를 수 있음).
- `upcoming` = 24시간 안 예고(아래 미니 예고 카드). 도넛 모양 · 문구 = `ring_of(남은 ms)`, 시각 = `when_label`.

### 8-2. 창 — `show(data, anchor)` · `update` · `follow` · `close` · `current` · `take_requests`
- 말풍선은 **하나만**. 레이어드 팝업(`WS_EX_LAYERED|WS_EX_TOOLWINDOW`, `UpdateLayeredWindow`), 꼬리 끝 = `Critters::anchor(member)`(캐릭터 머리 위 가운데, LIVE 중이면 배지 위). 폭 480px · 최대 높이 600px(위 공간이 모자라면 줄임) · 내용이 적으면 내용 높이.
- 같은 멤버로 다시 `show` = 닫기(캐릭터를 다시 누르면 닫힘). 다른 멤버 = 내용만 바꿈. 다른 곳을 누르면(비활성) · `Esc` 로 닫힘.
- 열기 320ms / 닫기 210ms — 꼬리 끝 기준 0.8 → 1 배 + 투명도(원본 PC 팝업과 같은 곡선 `cubic-bezier(0.2,0.8,0.2,1)`).
- 그림(`view.rs`): 바탕 = 멤버 색(아바타 평균색, 예고판 `sampleLaneColor`) 172deg 그라데이션, 글자색 = 검정 · 흰색 중 대비 큰 쪽, 번역 칸 바탕 = 글자 반대쪽(`paletteFor`).
  머리 줄 멤버 칩(지금 멤버 = 얼굴 + 이름 알약, 트윗 없는 멤버 = 흐리게) → (방송 중이면 「▶ 방송 보러 가기 · 제목」 알약) → 피드:
  날짜 구분선 · 글타래 세로줄 · 점 · 글 카드 · 면 점 2개. 위아래 끝 흐림 · 얇은 스크롤 막대 · ↓ 동그라미.
- **글 카드 두 면**(`card.rs`, 원본 `_article` · tweetlogic `XM` 치수) — 같은 칸에 겹치고 높이 = 큰 쪽.
  - 번역 면(.tws-ko): 얼굴 40 · 이름 · @핸들 · URL 복사 · 「이어서 · N분 뒤」 · 본문 15.5px/1.6(원문 본문 높이만큼 min-height — 사진 · 인용 위치 맞춤) · 미디어 격자 · 인용 상자(「인용한 글」 + 번역 + 미디어) · 시각 · X 원문.
  - 원문 면(X 카드 · 어둡게): 얼굴 · 이름 · @핸들 · X 로고 · 「@핸들 님에게 보내는 답글」 · 원문 20px/24px(주소 · #태그 · @멘션 파랑) · 미디어 · 인용(작성자 얼굴 · 이름 · @핸들 + 원문 15/20 + 미디어) · 「오후 7:55 · 2026년 9월 6일」 · 반응 줄(♡ 좋아요 수 · 답글 · 링크 복사) · 「답글 N개 읽기」 알약.
    이름 · 얼굴 · 숫자 · 인용 작성자는 그 면을 처음 펼칠 때 fxtwitter 에서 받는다(받기 전엔 데이터 값 · 「좋아요」 · 「X 에서 보기」).
  - 미디어 격자(.tws-md): 1칸 = 원래 비율(세로로 길면 정사각형) · 2~4칸 = 16:9 묶음 · 칸 사이 2 · 넷째 칸 「+N」 · 영상 ▶ · 원문 면 GIF 표시. 그림은 그 카드가 화면 근처에 오면 받는다.
  - **밀기**(원본 「겹쳐 밀기」): 카드나 점 줄을 가로로 8px 넘게 끌면 앞 면이 손을 따라 밀리며 흐려지고(1 − 1.2k), 뒤 면은 0.985 배 · 투명에서 나타난다(1.6k). 점 폭이 따라간다.
    놓을 때 25% 넘게 밀었으면 넘김(260ms, 앞 면은 ∓40% 로 나가며 사라짐), 아니면 제자리로 튕김(420ms, cubic-bezier(.25,1.4,.45,1)). 더 갈 면이 없는 쪽은 고무줄(최대 24px). 트랙패드 가로 휠 60 = 한 번 넘김(400ms 쉼).
  - 머리 줄 「번역 · 원문」 탭 = 모든 글 한꺼번에(보이는 글은 움직임, 알약이 미끄러짐) · ← → 키도 같음. 저장값 = 설정 `tweet_face_orig`(원본 mew:tllang).
  - 영상 칸(두 면 모두)을 누르면 영상 창(§8-5)에서 재생. 원문 면 사진 · X 로고 · 반응 줄 · 알약 = 브라우저로 트윗 열기, 「링크 복사」 = 주소 복사.
- **미니 예고 카드**(말풍선 맨 아래 띠, 원본 예고판 `.tcard` · `updateRing`): 이 멤버가 나오는(합동 포함) `upcoming`·`watching` 예고 중 24시간 안에 시작하는 가장 이른 것(시작 시각이 지나도 방송 전이면 3시간까지).
  썸네일(mqdefault, 회원 전용이면 🔒) · 「11/14(토) 21:00」 + 합동 · 회원 전용 태그 · 제목 한 줄 · 도넛 40px: 1시간 넘게 남음 = 24시간 중 지난 비율만큼 멤버 색 + 「n시간 / 시작까지」,
  1시간 이내 = 노란 링이 남은 비율만큼(12시부터 비워짐) + 「n분」, 지남 = 빈 링 + 노란 「지각」. 30초마다 갱신. 누르면 방송 주소.
- 그리기 구조: 바탕(그림자 · 그라데이션 · 꼬리)은 크기 · 색이 바뀔 때만, 카드 면은 한 장씩 그려 캐시(서명 = 올린 단추 · 복사 표시 · 받은 그림 · 원문 정보), 스크롤 · 밀기 땐 붙이기만.
  글자는 GDI 흑백 커버리지를 받아 색을 섞는다(반투명 · 움직임 위에서도 그릴 수 있게).
- 열면 맨 아래(최신). 휠 · ↑↓ · PgUp/PgDn · Home/End 로 부드럽게 스크롤. 폴링으로 내용이 바뀌면 `update`(맨 아래였으면 맨 아래 유지).
- **자동 말풍선** `show_auto(data, anchor)` (설정 `auto_bubble`, 폴링에서 새 트윗이 온 멤버 · 여럿이면 마지막 · 전체 화면 앱 중엔 안 띄움):
  초점을 빼앗지 않고 뜬 뒤 5초 뒤 닫힌다. 마우스가 올라오면 타이머를 멈추고, 나가면 5초를 처음부터. 누르면(또는 그 캐릭터를 누르면) 고정 = 평소 말풍선(바깥을 누르면 닫힘),
  고정되면 `take_pinned()` 로 알려 새 트윗 점을 지운다. 사용자가 열어 둔 말풍선이 있으면 덮지 않는다(같은 멤버면 내용만 갱신).
- **항상 위** `set_topmost(on)` · `Critters::set_topmost(on)`: 켜면 캐릭터 · 말풍선 모두 `HWND_TOPMOST`(전체 화면 앱 중엔 캐릭터를 숨김).
  끄면 캐릭터는 맨 아래, 자동 말풍선도 다른 창 뒤(맨 아래)에 뜬다. 사용자가 연 말풍선은 어느 쪽이든 맨 앞 + 초점.
- 메인 루프가 매 바퀴 `follow(anchor)`(캐릭터를 따라감) · `Critters::set_talking(current())`(말하는 동안 유노는 멈춰 섬) · `take_requests()`(칩을 누른 멤버로 다시 열기).

### 8-3a. 항상 위 (critter.rs)
- 켬 = 다섯 창 모두 `HWND_TOPMOST`(뒤에서부터 올려 첫 창이 무리 맨 아래에 깔리지 않게). 창의 z순서가 바뀌려 하면(`WM_WINDOWPOSCHANGING`) 설정 쪽으로 고정 —
  켬이면 항상 위 무리 맨 앞, 끔이면 맨 아래. 켬일 때 1초마다 다섯 창의 항상 위 플래그를 확인해 풀린 창이 있으면 로그(`[critter] 항상 위가 풀린 창`)를 남기고 다시 올린다.
  (2026-11-14 수정: 아라레만 항상 위가 풀려 다른 창 뒤로 가던 문제 — 예전 훅은 켬일 때 z순서 변경을 그대로 통과시켜, 보통 창 뒤로 넣는 요청이 오면 항상 위가 풀렸다.)

### 8-4. `net.rs` — 그림 · 원문 정보 받기
- 일꾼 스레드 4개(ureq). 그림 = pbs.twimg.com 은 `name=small`, 해독 후 긴 변 1200px 이하로 줄여 메인 스레드 캐시에. 원문 정보 = `https://api.fxtwitter.com/status/{id}`(vxtwitter 는 Cloudflare 403) → 작성자 · 얼굴 · 좋아요 · 답글 · 게시 시각 · 영상(mp4: 720p 이하 중 가장 좋은 것) · 인용 작성자 · 인용 영상.
- 다 받으면 등록된 창(말풍선 · 영상 창)에 `WM_APP_NET`. 같은 주소는 한 번만(실패도 기억).

### 8-5. `player.rs` — 영상 창
- 영상 칸을 누르면: 말풍선이 주인인 팝업(둥근 모서리 · 그림자)을 말풍선 가운데에 띄우고, fxtwitter 로 mp4 주소를 받아 Media Foundation(MFPlay)으로 재생(소리 켬, GIF = 소리 없이 반복).
- 아래 조작 줄 44px: 재생/멈춤 · 진행 막대(누르면 그 자리로) · 시각 · 소리 · 닫기. 영상을 누르면 재생/멈춤, Space · ← → (5초) · M · Esc. 끝나면 가운데 ▶(다시 보기).
- 실패하면 「영상을 불러오지 못했어요 · 누르면 X 에서 열어요」. 말풍선 · 영상 창 밖을 누르면 둘 다 닫힘.

### 8-3. 시험
- `build_data` · 글타래 · 시각 문구를 `#[cfg(test)]` 로.
- 그림 확인: `cargo test shot_bubble -- --ignored`(→ `target/bubble_*.png`), `cargo test critter_bubble -- --ignored`(캐릭터 + 말풍선 합성 → `target/critter_bubble_*.png`, 캐릭터 항상 위 전환), `cargo test auto_bubble -- --ignored`(자동 말풍선 5초 · 마우스 초기화 · 클릭 고정 · 항상 위 전환).
- 실데이터: `cargo test --release shot_bubble -- --ignored`(번역 · 원문 · 밀기 중간), `shot_media`(사진 · 원문 면 아래), `video_from_bubble`(영상 칸 → 재생 · 화면 캡처 `target/video_play.png`), `play_video`(영상 창 단독), `perf_bubble`(프레임 시간), `shot_mini`(미니 예고 카드: 실제 · 50분 · 지각).
- 항상 위: `cargo test topmost_all -- --ignored`(끔 → 켬 → 아라레를 맨 아래 · 항상 위 밖으로 밀어도 남는지 → 끔).
