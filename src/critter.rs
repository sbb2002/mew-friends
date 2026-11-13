//! 바탕화면 캐릭터 창. 명세: docs/IMPL_SPEC.md §7. [담당: 작업 C]
//!
//! 멤버마다 WS_POPUP 레이어드 창 하나. 그림은 UpdateLayeredWindow 로 픽셀 알파를 올린다.
//! 그리기 · 움직임 계산은 `rig` · `motion` 모듈(순수)이 맡고, 이 파일은 창 · DIB · 입력만 다룬다.
//! 모든 호출은 메인 스레드에서 한다(wndproc 도 메인 스레드의 메시지 루프에서 불린다).

mod motion;
mod rig;

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tiny_skia::Pixmap;
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ,
    ReleaseDC, SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Shell::{
    QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GWLP_USERDATA, GetWindowLongPtrW, HWND_BOTTOM, HWND_TOPMOST, IDC_HAND,
    LoadCursorW, MA_NOACTIVATE, RegisterClassExW, SET_WINDOW_POS_FLAGS, SetCursor,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, SPI_GETWORKAREA, SW_HIDE, SW_SHOWNOACTIVATE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW, UpdateLayeredWindow,
    ULW_ALPHA, WINDOW_EX_STYLE, WINDOWPOS, WM_DISPLAYCHANGE, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEACTIVATE, WM_SETCURSOR, WM_SETTINGCHANGE, WM_WINDOWPOSCHANGING, WNDCLASSEXW,
    WNDCLASS_STYLES, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_POPUP,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
};
use windows::core::{PCWSTR, w};

use self::motion::{Phase, Wander};

/// 창 폭 · 전체 높이(LIVE 자리 포함)의 기준값. 실제 크기 = 기준 × DPI 배율.
const WIN_W: f32 = 140.0;
const WIN_H: f32 = 158.0;
/// 창 위쪽 LIVE 표시 자리.
const LIVE_H: f32 = 28.0;
/// 말풍선 꼬리 끝 ~ 캐릭터 그림 칸 위쪽 사이(그림 위 투명 여백만큼 내린다).
const HEAD_GAP: f32 = 4.0;
/// 작업 영역을 나누는 칸 수(멤버 5명 · 유노 칸도 비워 둔다).
const SLOTS: f32 = 5.0;
/// 걷기 속도(기준 px/초 × 배율).
const WALK_SPEED: f64 = 40.0;

thread_local! {
    /// 눌렀다 뗀 창의 대기열(창 index).
    static CLICKS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    /// 왼쪽 버튼을 누른 창(창 index). 떼는 창과 같을 때만 클릭이다.
    static PRESSED: Cell<Option<usize>> = const { Cell::new(None) };
    /// 항상 위(트레이 설정). 끄면 다른 창 뒤 바탕화면에만 둔다.
    static TOPMOST: Cell<bool> = const { Cell::new(false) };
    /// 해상도 · 작업 영역이 바뀌었다는 표시(다음 tick 에서 다시 배치).
    static RELAYOUT: Cell<bool> = const { Cell::new(false) };
}

/// 그림 종류. 유노는 리그로 걷고, 나머지는 정지 그림이다.
enum Art {
    Yuno,
    Still(Option<Pixmap>),
}

/// GDI 32비트 DIB(화면 DC 와 호환). bits 는 BGRA premultiplied 를 받는다.
struct Dib {
    dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    len: usize,
}

impl Dib {
    fn new(w: i32, h: i32) -> Result<Dib, String> {
        unsafe {
            let screen = GetDC(None);
            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 위에서 아래로
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0, // BI_RGB
                ..Default::default()
            };
            let mut bits: *mut c_void = null_mut();
            let bmp = CreateDIBSection(
                Some(screen),
                &bmi as *const BITMAPINFO,
                DIB_RGB_COLORS,
                &mut bits as *mut *mut c_void,
                None,
                0,
            );
            let dc = CreateCompatibleDC(Some(screen));
            ReleaseDC(None, screen);
            let bmp = match bmp {
                Ok(b) => b,
                Err(e) => {
                    let _ = DeleteDC(dc);
                    return Err(format!("DIB 생성 실패: {e}"));
                }
            };
            let old = SelectObject(dc, HGDIOBJ(bmp.0));
            Ok(Dib { dc, bmp, old, bits: bits as *mut u8, len: (w * h * 4) as usize })
        }
    }

    fn bits_mut(&mut self) -> &mut [u8] {
        // SAFETY: CreateDIBSection 이 준 메모리를 bmp 가 살아 있는 동안 쓴다(같은 구조체 안).
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.len) }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.bmp.0));
            let _ = DeleteDC(self.dc);
        }
    }
}

/// 멤버 창 하나.
struct Win {
    key: String,
    hwnd: HWND,
    art: Art,
    pm: Pixmap,
    dib: Dib,
    live: bool,
    unread: bool,
    /// 다시 그릴 필요가 있다(정지 멤버는 바뀔 때만 그린다).
    dirty: bool,
    /// 정지 멤버의 가로 자리(작업 영역 5등분 가운데).
    slot_x: i32,
    /// 마지막으로 놓은 위치(같으면 SetWindowPos 를 생략).
    pos: Option<(i32, i32)>,
}

pub struct Critters {
    wins: Vec<Win>,
    sprites: rig::YunoSprites,
    badge: Option<Pixmap>,
    yuno: Option<usize>,
    wander: Option<Wander>,
    cv: rig::Canvas,
    scale: f32,
    win_w: i32,
    win_h: i32,
    work: RECT,
    start: Instant,
    last_t: Option<f64>,
    busy: bool,
    busy_checked: f64,
    /// 항상 위인데 전체 화면 앱이 떠 있어 창을 숨겼다
    hidden: bool,
    /// 항상 위 확인 시각
    top_checked: f64,
    frame_ms: u32,
    /// 말풍선을 띄운 멤버 — 유노면 걷기를 멈추고 제자리에서 쉰다.
    talking: Option<String>,
}

impl Critters {
    /// 멤버마다 창 하나. 메인 스레드에서 만든다.
    pub fn new(order: &[String]) -> Result<Critters, String> {
        let dpi = unsafe { GetDpiForSystem() };
        let scale = dpi.max(96) as f32 / 96.0;
        let win_w = (WIN_W * scale).round() as i32;
        let win_h = (WIN_H * scale).round() as i32;
        let cv = rig::Canvas {
            w: win_w as f32,
            h: win_h as f32,
            char_h: win_h as f32 - LIVE_H * scale,
        };
        let sprites = rig::YunoSprites::load()?;
        let badge = match rig::decode(rig::live_badge_bytes()) {
            Ok(b) => Some(b),
            Err(e) => {
                eprintln!("[critter] LIVE 배지 해독 실패: {e}");
                None
            }
        };
        let work = work_area();
        let hinst = module_instance();
        register_class(hinst)?;

        let mut wins = Vec::with_capacity(order.len());
        for (i, key) in order.iter().enumerate() {
            let hwnd = create_window(hinst, win_w, win_h)?;
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (i + 1) as isize);
            }
            let art = if key == "yuno" {
                Art::Yuno
            } else {
                let still = rig::still_bytes(key).and_then(|b| match rig::decode(b) {
                    Ok(p) => Some(p),
                    Err(e) => {
                        eprintln!("[critter] {key} 그림 해독 실패: {e}");
                        None
                    }
                });
                Art::Still(still)
            };
            let pm = Pixmap::new(win_w as u32, win_h as u32).ok_or("캔버스 생성 실패")?;
            let dib = Dib::new(win_w, win_h)?;
            wins.push(Win {
                key: key.clone(),
                hwnd,
                art,
                pm,
                dib,
                live: false,
                unread: false,
                dirty: true,
                slot_x: 0,
                pos: None,
            });
        }
        let yuno = order.iter().position(|k| k == "yuno");

        let mut c = Critters {
            wins,
            sprites,
            badge,
            yuno,
            wander: None,
            cv,
            scale,
            win_w,
            win_h,
            work,
            start: Instant::now(),
            last_t: None,
            busy: false,
            busy_checked: f64::NEG_INFINITY,
            hidden: false,
            top_checked: 0.0,
            frame_ms: 250,
            talking: None,
        };
        c.layout();
        if let Some(yi) = yuno {
            let x0 = c.wins[yi].slot_x as f64;
            c.wander = Some(Wander::new(seed(), 0.0, x0));
        }
        // 창 뒤 바탕화면에만: 맨 아래로 내린 뒤 보인다.
        for w in &c.wins {
            unsafe {
                let _ = SetWindowPos(
                    w.hwnd,
                    Some(HWND_BOTTOM),
                    0,
                    0,
                    0,
                    0,
                    flags(&[SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE]),
                );
            }
        }
        c.tick();
        for w in &c.wins {
            unsafe {
                let _ = ShowWindow(w.hwnd, SW_SHOWNOACTIVATE);
            }
        }
        Ok(c)
    }

    /// 시계 기준으로 움직임을 진행하고 필요하면 다시 그린다.
    pub fn tick(&mut self) {
        let now = self.start.elapsed().as_secs_f64();
        if now - self.busy_checked >= 1.0 {
            self.busy_checked = now;
            self.busy = fullscreen_busy();
        }
        // 항상 위인데 풀린 창이 있으면 다시 올린다(1초마다 확인 — 무엇이 풀었는지는 로그로)
        let top = TOPMOST.with(|t| t.get());
        if top && !self.hidden && now - self.top_checked >= 1.0 {
            self.top_checked = now;
            let lost: Vec<&str> = self.wins.iter().filter(|w| !is_topmost(w.hwnd)).map(|w| w.key.as_str()).collect();
            if !lost.is_empty() {
                eprintln!("[critter] 항상 위가 풀린 창: {lost:?} → 다시 올림");
                self.set_topmost(true);
            }
        }
        // 항상 위일 때 전체 화면 앱(게임 · 발표) 위로 올라오지 않게 숨긴다.
        let hide = self.busy && top;
        if hide != self.hidden {
            self.hidden = hide;
            for w in &self.wins {
                unsafe {
                    let _ = ShowWindow(w.hwnd, if hide { SW_HIDE } else { SW_SHOWNOACTIVATE });
                }
            }
        }
        if self.busy {
            // 전체 화면 · 프레젠테이션 중엔 그리지도 움직이지도 않는다.
            self.frame_ms = 1000;
            self.last_t = None;
            return;
        }
        if RELAYOUT.with(|r| r.replace(false)) {
            self.layout();
        }
        let dt = self.last_t.map_or(0.0, |t| (now - t).clamp(0.0, 0.25));
        self.last_t = Some(now);

        // 유노: 걷기 · 쉬기 상태 → 위치 · 포즈 → 그림
        let mut yuno_phase = None;
        if let (Some(yi), Some(_)) = (self.yuno, self.wander.as_ref()) {
            let min = self.work.left as f64;
            let max = (self.work.right - self.win_w) as f64;
            let y = self.win_y();
            let speed = WALK_SPEED * self.scale as f64;
            let hold = self.talking.as_deref() == Some(self.wins[yi].key.as_str());
            if let Some(wd) = self.wander.as_mut() {
                if !hold {
                    wd.step(now, dt, speed, min, max);
                }
                let phase = if hold { Phase::Idle } else { wd.phase };
                let (x, flip) = (wd.x.round() as i32, wd.flip() as f32);
                yuno_phase = Some(phase);
                let pose = motion::pose(phase, now);
                let closed = motion::blink_closed(now);
                let w = &mut self.wins[yi];
                place(w, x, y);
                rig::yuno_frame(&mut w.pm, &self.sprites, &pose, closed, &self.cv, flip);
                let badge = if w.live { self.badge.as_ref() } else { None };
                rig::draw_overlays(&mut w.pm, badge, w.unread, &self.cv, self.scale);
                present(w.hwnd, &w.pm, &mut w.dib);
            }
        }

        // 정지 멤버: 바뀌었을 때만. LIVE 중인 멤버는 2초 주기로 아주 약하게 움직인다.
        let pulse = ((2.0 * std::f64::consts::PI * now) / 2.0).sin() as f32 * 2.0 * self.scale;
        let cv = self.cv;
        let scale = self.scale;
        let badge = self.badge.as_ref();
        for w in self.wins.iter_mut() {
            if !matches!(w.art, Art::Still(_)) || !(w.dirty || w.live) {
                continue;
            }
            w.dirty = false;
            let bounce = if w.live { pulse } else { 0.0 };
            let show_badge = if w.live { badge } else { None };
            let Art::Still(Some(img)) = &w.art else {
                continue;
            };
            rig::still_frame(&mut w.pm, img, &cv, bounce);
            rig::draw_overlays(&mut w.pm, show_badge, w.unread, &cv, scale);
            present(w.hwnd, &w.pm, &mut w.dib);
        }

        // 다음 간격: 걷기 50ms · 쉬기 또는 LIVE 표시 중 100ms · 그 밖 250ms
        let walking = yuno_phase == Some(Phase::Walk);
        let idle = yuno_phase == Some(Phase::Idle);
        let any_live = self.wins.iter().any(|w| w.live);
        self.frame_ms = if walking {
            50
        } else if idle || any_live {
            100
        } else {
            250
        };
    }

    /// 다음 tick 까지 원하는 간격(ms).
    pub fn frame_interval_ms(&self) -> u32 {
        self.frame_ms
    }

    pub fn set_live(&mut self, member: &str, live: bool) {
        if let Some(w) = self.wins.iter_mut().find(|w| w.key == member) {
            if w.live != live {
                w.live = live;
                w.dirty = true;
            }
        }
    }

    pub fn set_unread(&mut self, member: &str, unread: bool) {
        if let Some(w) = self.wins.iter_mut().find(|w| w.key == member) {
            if w.unread != unread {
                w.unread = unread;
                w.dirty = true;
            }
        }
    }

    /// 전체 화면 앱 · 프레젠테이션 중(자동 말풍선을 띄우지 않는다).
    pub fn is_busy(&self) -> bool {
        self.busy
    }

    /// 항상 위 켜기 · 끄기. 끄면 맨 아래(바탕화면 바로 위)로 내린다.
    pub fn set_topmost(&mut self, on: bool) {
        TOPMOST.with(|t| t.set(on));
        // 뒤에서부터 올린다(먼저 올린 창이 무리 맨 아래에 깔리므로 — 순서대로면 아라레가 늘 가장 아래였다)
        for w in self.wins.iter().rev() {
            unsafe {
                let _ = SetWindowPos(
                    w.hwnd,
                    Some(if on { HWND_TOPMOST } else { HWND_BOTTOM }),
                    0,
                    0,
                    0,
                    0,
                    flags(&[SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE]),
                );
            }
        }
    }

    /// 말풍선을 띄운 멤버(없으면 None). 유노는 말하는 동안 멈춰 선다.
    pub fn set_talking(&mut self, member: Option<&str>) {
        self.talking = member.map(str::to_string);
    }

    /// 말풍선 꼬리가 가리킬 점(화면 좌표) — 캐릭터 머리 위 가운데. LIVE 배지가 있으면 배지 위.
    pub fn anchor(&self, member: &str) -> Option<(i32, i32)> {
        let w = self.wins.iter().find(|w| w.key == member)?;
        let (x, y) = w.pos?;
        let top = if w.live { 0.0 } else { LIVE_H * self.scale } + HEAD_GAP * self.scale;
        Some((x + self.win_w / 2, y + top.round() as i32))
    }

    /// 지난 호출 뒤 눌린 캐릭터(멤버 key).
    pub fn take_clicks(&mut self) -> Vec<String> {
        let idx: Vec<usize> = CLICKS.with(|c| c.borrow_mut().drain(..).collect());
        idx.into_iter()
            .filter_map(|i| self.wins.get(i).map(|w| w.key.clone()))
            .collect()
    }

    /// 창 아랫변 = 작업 영역 아랫선 − 2px.
    fn win_y(&self) -> i32 {
        self.work.bottom - 2 - self.win_h
    }

    /// 작업 영역을 다시 읽고 정지 멤버를 칸 가운데에 놓는다.
    fn layout(&mut self) {
        self.work = work_area();
        let ww = (self.work.right - self.work.left) as f32;
        let y = self.win_y();
        let win_w = self.win_w as f32;
        for (k, w) in self.wins.iter_mut().enumerate() {
            w.slot_x = (self.work.left as f32 + (k as f32 + 0.5) * ww / SLOTS - win_w / 2.0)
                .round() as i32;
            if matches!(w.art, Art::Still(_)) {
                w.dirty = true;
                place(w, w.slot_x, y);
            }
        }
    }
}

/// 그림을 창에 올린다: pm(RGBA premultiplied) → DIB(BGRA) → UpdateLayeredWindow.
fn present(hwnd: HWND, pm: &Pixmap, dib: &mut Dib) {
    let dc = dib.dc;
    {
        let dst = dib.bits_mut();
        for (s, d) in pm.data().chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
            d[0] = s[2];
            d[1] = s[1];
            d[2] = s[0];
            d[3] = s[3];
        }
    }
    let size = SIZE { cx: pm.width() as i32, cy: pm.height() as i32 };
    let src_pt = POINT { x: 0, y: 0 };
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    unsafe {
        let screen = GetDC(None);
        if let Err(e) = UpdateLayeredWindow(
            hwnd,
            Some(screen),
            None, // 위치는 SetWindowPos 가 맡는다
            Some(&size as *const SIZE),
            Some(dc),
            Some(&src_pt as *const POINT),
            COLORREF(0),
            Some(&blend as *const BLENDFUNCTION),
            ULW_ALPHA,
        ) {
            eprintln!("[critter] UpdateLayeredWindow 실패: {e}");
        }
        ReleaseDC(None, screen);
    }
}

/// 위치가 바뀔 때만 옮긴다(z 순서 · 활성화는 건드리지 않는다).
fn place(w: &mut Win, x: i32, y: i32) {
    if w.pos == Some((x, y)) {
        return;
    }
    w.pos = Some((x, y));
    unsafe {
        if let Err(e) = SetWindowPos(
            w.hwnd,
            None,
            x,
            y,
            0,
            0,
            flags(&[SWP_NOZORDER, SWP_NOACTIVATE, SWP_NOSIZE]),
        ) {
            eprintln!("[critter] 창 위치 설정 실패: {e}");
        }
    }
}

fn flags(list: &[SET_WINDOW_POS_FLAGS]) -> SET_WINDOW_POS_FLAGS {
    SET_WINDOW_POS_FLAGS(list.iter().fold(0, |acc, f| acc | f.0))
}

/// 주 모니터 작업 영역(작업표시줄 제외). 실패하면 1920×1080 으로 둔다.
fn work_area() -> RECT {
    let mut rc = RECT::default();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut rc as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok();
    if !ok || rc.right <= rc.left || rc.bottom <= rc.top {
        eprintln!("[critter] 작업 영역 조회 실패 — 1920×1080 으로 대신한다");
        rc = RECT { left: 0, top: 0, right: 1920, bottom: 1080 };
    }
    rc
}

/// 전체 화면 앱 · 프레젠테이션 중인지.
fn fullscreen_busy() -> bool {
    match unsafe { SHQueryUserNotificationState() } {
        Ok(s) => {
            let v = s.0;
            v == QUNS_BUSY.0 || v == QUNS_RUNNING_D3D_FULL_SCREEN.0 || v == QUNS_PRESENTATION_MODE.0
        }
        Err(_) => false,
    }
}

fn module_instance() -> HINSTANCE {
    match unsafe { GetModuleHandleW(PCWSTR::null()) } {
        Ok(m) => HINSTANCE(m.0),
        Err(_) => HINSTANCE(null_mut()),
    }
}

fn register_class(hinst: HINSTANCE) -> Result<(), String> {
    let class = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: WNDCLASS_STYLES(0),
        lpfnWndProc: Some(wndproc),
        hInstance: hinst,
        lpszClassName: w!("MewFriendsCritter"),
        ..Default::default()
    };
    if unsafe { RegisterClassExW(&class as *const WNDCLASSEXW) } == 0 {
        return Err("캐릭터 창 클래스 등록 실패".to_string());
    }
    Ok(())
}

fn create_window(hinst: HINSTANCE, w: i32, h: i32) -> Result<HWND, String> {
    let ex = WINDOW_EX_STYLE(WS_EX_LAYERED.0 | WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0);
    unsafe {
        CreateWindowExW(
            ex,
            w!("MewFriendsCritter"),
            w!(""),
            WS_POPUP,
            0,
            0,
            w,
            h,
            None,
            None,
            Some(hinst),
            None,
        )
    }
    .map_err(|e| format!("캐릭터 창 생성 실패: {e}"))
}

fn is_topmost(hwnd: HWND) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongW, WS_EX_TOPMOST};
    unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 }
}

/// 창 userdata 에 넣어 둔 index(i + 1). 0 이면 없음.
fn index_of(hwnd: HWND) -> Option<usize> {
    let v = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    if v <= 0 { None } else { Some((v - 1) as usize) }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_WINDOWPOSCHANGING => {
                // z 순서를 바꾸려는 시도(NOZORDER 가 없음)는 설정 쪽으로 고친다:
                // 항상 위 = 항상 위 무리 맨 앞(다른 무엇이 끌어내리려 해도 · 활성화돼도), 끔 = 맨 아래.
                if lp.0 != 0 {
                    let pos = lp.0 as *mut WINDOWPOS;
                    if (*pos).flags.0 & SWP_NOZORDER.0 == 0 {
                        let top = TOPMOST.with(|t| t.get());
                        let want = if top { HWND_TOPMOST } else { HWND_BOTTOM };
                        // (Windows 는 HWND_TOPMOST 요청도 실제 창 핸들로 바꿔 보내므로 요청 값으로는 판단하지 않는다)
                        (*pos).hwndInsertAfter = want;
                    }
                }
            }
            WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
            WM_LBUTTONDOWN => {
                if let Some(i) = index_of(hwnd) {
                    PRESSED.with(|p| p.set(Some(i)));
                }
            }
            WM_LBUTTONUP => {
                let pressed = PRESSED.with(|p| p.replace(None));
                if let (Some(i), Some(p)) = (index_of(hwnd), pressed) {
                    if i == p {
                        CLICKS.with(|c| c.borrow_mut().push(i));
                    }
                }
            }
            WM_SETCURSOR => {
                if let Ok(cur) = LoadCursorW(None, IDC_HAND) {
                    SetCursor(Some(cur));
                }
                return LRESULT(1);
            }
            WM_DISPLAYCHANGE | WM_SETTINGCHANGE => {
                RELAYOUT.with(|r| r.set(true));
            }
            _ => {}
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn seed() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
}

#[cfg(test)]
mod bubble_tests {
    use super::*;
    use crate::model::{parse_preview, parse_tweets};
    use tiny_skia::{PixmapPaint, Transform};

    /// 캐릭터 + 말풍선을 화면 위치 그대로 합쳐 `target/critter_bubble.png` 로 저장한다.
    /// 실행: cargo test critter_bubble -- --ignored --nocapture
    #[test]
    #[ignore]
    fn critter_bubble() {
        let order: Vec<String> = ["arale", "yuno", "nonoka", "ritsu", "miyako"].iter().map(|s| s.to_string()).collect();
        let mut c = Critters::new(&order).unwrap();
        let top = |h: HWND| unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowLongW(h, windows::Win32::UI::WindowsAndMessaging::GWL_EXSTYLE) } as u32
            & windows::Win32::UI::WindowsAndMessaging::WS_EX_TOPMOST.0 != 0;
        c.set_topmost(true);
        assert!(c.wins.iter().all(|w| top(w.hwnd)), "항상 위 켜짐");
        c.set_topmost(false);
        assert!(c.wins.iter().all(|w| !top(w.hwnd)), "항상 위 꺼짐");
        c.set_live("nonoka", true);
        c.tick();
        let p = parse_preview(include_str!("../fixtures/preview.sample.json")).unwrap();
        let t = parse_tweets(include_str!("../fixtures/tweets.sample.json")).unwrap();
        let now = crate::util::parse_utc("2026-09-07T12:30:00Z").unwrap();
        for key in ["yuno", "nonoka", "arale"] {
            c.set_talking(Some(key));
            c.tick();
            let a = c.anchor(key).unwrap();
            crate::panel::show(crate::panel::build_data(key, &p, &t, now), a);
            let (img, pos) = crate::panel::snapshot().unwrap();
            let w = c.wins.iter().find(|w| w.key == key).unwrap();
            let (wx, wy) = w.pos.unwrap();
            let left = pos.0.min(wx);
            let top = pos.1;
            let right = (pos.0 + img.width() as i32).max(wx + c.win_w);
            let bottom = wy + c.win_h;
            let mut out = Pixmap::new((right - left) as u32, (bottom - top) as u32).unwrap();
            out.fill(tiny_skia::Color::from_rgba8(58, 110, 165, 255));
            out.draw_pixmap(wx - left, wy - top, w.pm.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            out.draw_pixmap(pos.0 - left, pos.1 - top, img.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("target/critter_bubble_{key}.png"));
            out.save_png(&path).unwrap();
            println!("saved {} anchor={a:?} win=({wx},{wy})", path.display());
            crate::panel::close();
            let start = std::time::Instant::now();
            while start.elapsed().as_millis() < 400 {
                unsafe {
                    let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
                    while windows::Win32::UI::WindowsAndMessaging::PeekMessageW(&mut msg, None, 0, 0, windows::Win32::UI::WindowsAndMessaging::PM_REMOVE).as_bool() {
                        windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
}

#[cfg(test)]
mod topmost_tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongW, WS_EX_TOPMOST};

    fn tops(c: &Critters) -> Vec<(String, bool)> {
        c.wins
            .iter()
            .map(|w| (w.key.clone(), unsafe { GetWindowLongW(w.hwnd, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0 != 0))
            .collect()
    }

    /// 끔으로 시작 → 켬 → tick 몇 번 — 모든 캐릭터가 항상 위여야 한다.
    #[test]
    #[ignore]
    fn topmost_all() {
        let order: Vec<String> = ["arale", "yuno", "nonoka", "ritsu", "miyako"].iter().map(|s| s.to_string()).collect();
        let mut c = Critters::new(&order).unwrap();
        c.set_topmost(false);
        println!("off  : {:?}", tops(&c));
        c.set_topmost(true);
        println!("on   : {:?}", tops(&c));
        c.set_live("arale", true);
        for _ in 0..5 {
            c.tick();
        }
        println!("tick : {:?}", tops(&c));
        assert!(tops(&c).iter().all(|(_, t)| *t));
        // 누군가 아라레를 맨 아래 · 항상 위 밖으로 옮기려 해도 항상 위로 남는다
        unsafe {
            let _ = SetWindowPos(c.wins[0].hwnd, Some(HWND_BOTTOM), 0, 0, 0, 0, flags(&[SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE]));
            let _ = SetWindowPos(c.wins[0].hwnd, Some(windows::Win32::UI::WindowsAndMessaging::HWND_NOTOPMOST), 0, 0, 0, 0, flags(&[SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE]));
        }
        println!("poke : {:?}", tops(&c));
        assert!(tops(&c)[0].1, "아라레가 항상 위로 남아야 한다");
        // 끄면 다 내려간다
        c.set_topmost(false);
        assert!(tops(&c).iter().all(|(_, t)| !*t));
    }
}
