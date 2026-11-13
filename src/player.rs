//! 영상 창 — 트윗 속 영상 · GIF 를 위젯 안에서 재생한다. 명세: docs/IMPL_SPEC.md §8-5.
//!
//! 데이터에는 썸네일 주소만 있으므로 누르면 fxtwitter 에서 mp4 주소를 받아(`net::fx`) Media Foundation(MFPlay)으로 튼다.
//! 창 = 말풍선이 주인인 팝업 하나: 위 = 영상(자식 창), 아래 = 조작 줄(재생 · 진행 막대 · 시각 · 소리 · 닫기).
//! GIF 는 소리 없이 반복. 실패하면 「X에서 열기」를 안내한다.

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::size_of;
use std::sync::Once;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH,
    DT_CENTER, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DT_WORDBREAK, DeleteDC,
    DeleteObject, DrawTextW, EndPaint, Ellipse, FF_DONTCARE, FW_BOLD, FW_NORMAL, FillRect,
    HFONT, HGDIOBJ, InvalidateRect, LineTo, MoveToEx, PAINTSTRUCT, PS_SOLID, Polygon,
    RoundRect, SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    OUT_DEFAULT_PRECIS, GetStockObject, NULL_PEN,
};
use windows::Win32::Media::MediaFoundation::{
    IMFPMediaPlayer, IMFPMediaPlayerCallback, MFP_CREATION_OPTIONS, MFP_MEDIAPLAYER_STATE_PAUSED,
    MFP_MEDIAPLAYER_STATE_PLAYING, MFP_MEDIAPLAYER_STATE_STOPPED, MFP_POSITIONTYPE_100NS,
    MFPCreateMediaPlayer,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_ESCAPE, VK_LEFT, VK_RIGHT, VK_SPACE};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DROPSHADOW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, IDC_ARROW,
    IDC_HAND, IsWindow, KillTimer, LoadCursorW, MoveWindow, RegisterClassExW, SPI_GETWORKAREA,
    SW_SHOW, SWP_NOACTIVATE, SWP_NOZORDER, SetCursor, SetForegroundWindow, SetTimer, SetWindowPos,
    ShowWindow, SystemParametersInfoW, WA_INACTIVE, WINDOW_EX_STYLE, WM_ACTIVATE, WM_DESTROY,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WM_SIZE,
    WM_TIMER, WNDCLASSEXW, WNDCLASS_STYLES, WS_CHILD, WS_CLIPCHILDREN, WS_EX_TOOLWINDOW,
    WS_POPUP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::net::{self, Slot};
use crate::util::{open_url, wide};

/// 재생 요청.
#[derive(Debug, Clone)]
pub struct Req {
    pub tweet_id: String,
    /// 인용 글 속 영상
    pub quote: bool,
    /// 누른 칸의 썸네일 주소(fxtwitter 영상과 맞춘다)
    pub thumb: String,
    /// 같은 묶음 안 영상 순번(썸네일로 못 찾을 때)
    pub idx: usize,
    /// 실패 때 열 주소(트윗)
    pub url: String,
    /// 말풍선(주인 창)
    pub owner: HWND,
    /// 말풍선 화면 위치(가운데에 띄운다)
    pub near: RECT,
}

#[derive(Debug, Clone, PartialEq)]
enum Status {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Btn {
    Play,
    Seek,
    Mute,
    Close,
}

struct Player {
    hwnd: HWND,
    video: HWND,
    req: Req,
    s: f32,
    status: Status,
    mf: Option<IMFPMediaPlayer>,
    gif: bool,
    aspect: f32,
    sized: bool,
    started: bool,
    ended: bool,
    paused: bool,
    muted: bool,
    pos: f64,
    dur: f64,
    hover: Option<Btn>,
    font: HFONT,
    font_b: HFONT,
}

thread_local! {
    static PLAYER: RefCell<Option<Player>> = const { RefCell::new(None) };
}

const BAR_H: f32 = 44.0;
const TIMER_POLL: usize = 1;
const X_BLUE: COLORREF = COLORREF(0xf0 << 16 | 0x9b << 8 | 0x1d);

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None) }.map(|m| HINSTANCE(m.0)).unwrap_or_default()
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("MewFriendsPlayer"),
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let vc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: WNDCLASS_STYLES(0),
            lpfnWndProc: Some(video_proc),
            hInstance: hinstance(),
            hCursor: LoadCursorW(None, IDC_HAND).unwrap_or_default(),
            lpszClassName: w!("MewFriendsVideo"),
            ..Default::default()
        };
        RegisterClassExW(&vc);
    });
}

fn font(size: f32, bold: bool) -> HFONT {
    unsafe {
        CreateFontW(
            -(size.round() as i32),
            0,
            0,
            0,
            if bold { FW_BOLD.0 as i32 } else { FW_NORMAL.0 as i32 },
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            (DEFAULT_PITCH.0 as u32) | (FF_DONTCARE.0 as u32),
            w!("Malgun Gothic"),
        )
    }
}

fn work_area() -> RECT {
    let mut r = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut r as *mut RECT as *mut c_void), Default::default());
    }
    r
}

/// 영상 비율에 맞춘 창 위치 · 크기(말풍선 가운데, 작업 영역 안).
fn frame_rect(near: RECT, aspect: f32, s: f32) -> RECT {
    let work = work_area();
    let bar = BAR_H * s;
    let max_w = (640.0 * s).min((work.right - work.left) as f32 * 0.9);
    let max_h = (work.bottom - work.top) as f32 * 0.85;
    let mut w = max_w;
    let mut h = w / aspect.max(0.2) + bar;
    if h > max_h {
        h = max_h;
        w = (h - bar) * aspect;
    }
    let (w, h) = (w.round() as i32, h.round() as i32);
    let cx = (near.left + near.right) / 2;
    let cy = (near.top + near.bottom) / 2;
    let x = (cx - w / 2).clamp(work.left, (work.right - w).max(work.left));
    let y = (cy - h / 2).clamp(work.top, (work.bottom - h).max(work.top));
    RECT { left: x, top: y, right: x + w, bottom: y + h }
}

/// 이 창이 영상 창인가(말풍선이 초점을 잃어도 닫지 않으려고).
pub fn is_player(hwnd: HWND) -> bool {
    PLAYER.with(|c| c.borrow().as_ref().is_some_and(|p| p.hwnd == hwnd || p.video == hwnd))
}

/// 영상을 튼다(이미 떠 있으면 바꾼다).
pub fn play(req: Req) {
    close();
    ensure_class();
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let s = (unsafe { GetDpiForSystem() } as f32 / 96.0).max(0.5);
    let rc = frame_rect(req.near, 16.0 / 9.0, s);
    let hwnd = unsafe {
        match CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("MewFriendsPlayer"),
            w!("Mew Friends 영상"),
            WS_POPUP | WS_CLIPCHILDREN,
            rc.left,
            rc.top,
            rc.right - rc.left,
            rc.bottom - rc.top,
            Some(req.owner),
            None,
            Some(hinstance()),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[player] 창 생성 실패: {e}");
                return;
            }
        }
    };
    let video = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("MewFriendsVideo"),
            w!(""),
            WS_CHILD | WS_VISIBLE,
            0,
            0,
            1,
            1,
            Some(hwnd),
            None,
            Some(hinstance()),
            None,
        )
        .unwrap_or_default()
    };
    unsafe {
        // 둥근 모서리(Windows 11) — DWMWCP_ROUND = 2
        let pref: u32 = 2;
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const u32 as *const c_void, 4);
    }
    PLAYER.with(|c| {
        *c.borrow_mut() = Some(Player {
            hwnd,
            video,
            req,
            s,
            status: Status::Loading,
            mf: None,
            gif: false,
            aspect: 16.0 / 9.0,
            sized: false,
            started: false,
            ended: false,
            paused: false,
            muted: false,
            pos: 0.0,
            dur: 0.0,
            hover: None,
            font: font(13.0 * s, false),
            font_b: font(13.0 * s, true),
        })
    });
    net::register(hwnd);
    layout(hwnd);
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetTimer(Some(hwnd), TIMER_POLL, 200, None);
    }
    poll();
}

/// 시험용: (재생 시작함, 위치 초)
#[cfg(test)]
pub fn debug_state() -> Option<(bool, f64)> {
    PLAYER.with(|c| c.borrow().as_ref().map(|p| (p.started, p.pos)))
}

/// 시험용: 영상 창 화면 위치
#[cfg(test)]
pub fn rect() -> Option<RECT> {
    let h = PLAYER.with(|c| c.borrow().as_ref().map(|p| p.hwnd))?;
    let mut rc = RECT::default();
    unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(h, &mut rc).ok()? };
    Some(rc)
}

pub fn close() {
    let h = PLAYER.with(|c| c.borrow().as_ref().map(|p| p.hwnd));
    if let Some(h) = h {
        unsafe {
            if IsWindow(Some(h)).as_bool() {
                let _ = DestroyWindow(h);
            } else {
                PLAYER.with(|c| *c.borrow_mut() = None);
            }
        }
    }
}

fn bar_h(s: f32) -> i32 {
    (BAR_H * s).round() as i32
}

fn layout(hwnd: HWND) {
    let Some((video, s, mf)) = PLAYER.with(|c| c.borrow().as_ref().map(|p| (p.video, p.s, p.mf.clone()))) else {
        return;
    };
    let mut rc = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rc);
        let _ = MoveWindow(video, 0, 0, rc.right, (rc.bottom - bar_h(s)).max(1), true);
        if let Some(mf) = mf {
            let _ = mf.UpdateVideo();
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}

fn secs(pv: &PROPVARIANT) -> f64 {
    i64::try_from(pv).map(|v| v as f64 / 1e7).unwrap_or(0.0)
}

/// 상태를 읽고(받기 · 재생 위치 · 끝) 필요하면 시작한다. 타이머 · 받기 알림마다.
fn poll() {
    net::drain();
    // 1) mp4 주소 기다리기
    let start = PLAYER.with(|c| {
        let mut g = c.borrow_mut();
        let p = g.as_mut()?;
        if p.status != Status::Loading {
            return None;
        }
        match net::fx(&p.req.tweet_id) {
            Slot::Loading => None,
            Slot::Failed => {
                p.status = Status::Failed("영상 정보를 받지 못했어요".into());
                None
            }
            Slot::Ready(fx) => {
                let list = if p.req.quote { &fx.quote_videos } else { &fx.videos };
                match net::match_video(list, &p.req.thumb, p.req.idx) {
                    Some(v) => {
                        p.gif = v.gif;
                        p.muted = v.gif;
                        if v.w > 0 && v.h > 0 {
                            p.aspect = v.w as f32 / v.h as f32;
                        }
                        p.status = Status::Ready;
                        Some((p.hwnd, p.video, v.mp4.clone(), p.req.near, p.aspect, p.s, p.gif))
                    }
                    None => {
                        p.status = Status::Failed("이 글에서 영상을 찾지 못했어요".into());
                        None
                    }
                }
            }
        }
    });
    if let Some((hwnd, video, url, near, aspect, s, gif)) = start {
        let rc = frame_rect(near, aspect, s);
        unsafe {
            let _ = SetWindowPos(hwnd, None, rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        let u = wide(&url);
        let mut mf: Option<IMFPMediaPlayer> = None;
        let r = unsafe {
            MFPCreateMediaPlayer(
                PCWSTR(u.as_ptr()),
                true,
                MFP_CREATION_OPTIONS(0),
                None::<&IMFPMediaPlayerCallback>,
                Some(video),
                Some(&mut mf),
            )
        };
        PLAYER.with(|c| {
            if let Some(p) = c.borrow_mut().as_mut() {
                match (r, mf) {
                    (Ok(()), Some(m)) => {
                        if gif {
                            unsafe {
                                let _ = m.SetMute(true);
                            }
                        }
                        p.mf = Some(m);
                        p.sized = true;
                    }
                    (e, _) => {
                        eprintln!("[player] 재생 시작 실패: {e:?}");
                        p.status = Status::Failed("영상을 재생하지 못했어요".into());
                    }
                }
            }
        });
        layout(hwnd);
    }

    // 2) 재생 상태
    let hwnd = PLAYER.with(|c| {
        let mut g = c.borrow_mut();
        let p = g.as_mut()?;
        if let Some(mf) = &p.mf {
            unsafe {
                if let Ok(st) = mf.GetState() {
                    if st == MFP_MEDIAPLAYER_STATE_PLAYING {
                        p.started = true;
                        p.ended = false;
                        p.paused = false;
                    } else if st == MFP_MEDIAPLAYER_STATE_PAUSED {
                        p.paused = true;
                    } else if st == MFP_MEDIAPLAYER_STATE_STOPPED && p.started && !p.paused {
                        // 끝까지 갔다 — GIF 는 처음부터 다시
                        if p.gif {
                            let zero = PROPVARIANT::from(0i64);
                            let _ = mf.SetPosition(&MFP_POSITIONTYPE_100NS, &zero);
                            let _ = mf.Play();
                        } else {
                            p.ended = true;
                        }
                    }
                }
                if let Ok(d) = mf.GetDuration(&MFP_POSITIONTYPE_100NS) {
                    p.dur = secs(&d);
                }
                if !p.ended {
                    if let Ok(pv) = mf.GetPosition(&MFP_POSITIONTYPE_100NS) {
                        p.pos = secs(&pv);
                    }
                } else {
                    p.pos = p.dur;
                }
            }
        }
        Some(p.hwnd)
    });
    if let Some(h) = hwnd {
        unsafe {
            let _ = InvalidateRect(Some(h), None, false);
        }
        let v = PLAYER.with(|c| c.borrow().as_ref().map(|p| p.video));
        if let Some(v) = v {
            unsafe {
                let _ = InvalidateRect(Some(v), None, false);
            }
        }
    }
}

fn toggle_play() {
    PLAYER.with(|c| {
        let mut g = c.borrow_mut();
        let Some(p) = g.as_mut() else { return };
        if let Status::Failed(_) = p.status {
            open_url(&p.req.url);
            return;
        }
        let Some(mf) = &p.mf else { return };
        unsafe {
            if p.ended {
                let zero = PROPVARIANT::from(0i64);
                let _ = mf.SetPosition(&MFP_POSITIONTYPE_100NS, &zero);
                let _ = mf.Play();
                p.ended = false;
            } else if p.paused || !p.started {
                let _ = mf.Play();
                p.paused = false;
            } else {
                let _ = mf.Pause();
                p.paused = true;
            }
        }
    });
}

fn seek_to(sec: f64) {
    PLAYER.with(|c| {
        let mut g = c.borrow_mut();
        let Some(p) = g.as_mut() else { return };
        let Some(mf) = &p.mf else { return };
        let t = sec.clamp(0.0, p.dur.max(0.0));
        unsafe {
            let v = PROPVARIANT::from((t * 1e7) as i64);
            let _ = mf.SetPosition(&MFP_POSITIONTYPE_100NS, &v);
            if p.ended {
                let _ = mf.Play();
                p.ended = false;
            }
        }
        p.pos = t;
    });
}

fn toggle_mute() {
    PLAYER.with(|c| {
        let mut g = c.borrow_mut();
        let Some(p) = g.as_mut() else { return };
        p.muted = !p.muted;
        if let Some(mf) = &p.mf {
            unsafe {
                let _ = mf.SetMute(p.muted);
            }
        }
    });
}

/// 조작 줄 칸(창 좌표).
struct BarRects {
    play: RECT,
    seek: RECT,
    time: RECT,
    mute: RECT,
    close: RECT,
}

fn bar_rects(cw: i32, ch: i32, s: f32, time_w: i32) -> BarRects {
    let b = bar_h(s);
    let y = ch - b;
    let px = |v: f32| (v * s).round() as i32;
    let play = RECT { left: px(6.0), top: y, right: px(6.0) + b, bottom: ch };
    let close = RECT { left: cw - px(6.0) - b, top: y, right: cw - px(6.0), bottom: ch };
    let mute = RECT { left: close.left - b, top: y, right: close.left, bottom: ch };
    let time = RECT { left: mute.left - time_w - px(8.0), top: y, right: mute.left - px(4.0), bottom: ch };
    let seek = RECT { left: play.right + px(6.0), top: y, right: time.left - px(10.0), bottom: ch };
    BarRects { play, seek, time, mute, close }
}

fn fmt_t(t: f64) -> String {
    let t = t.max(0.0).round() as u64;
    format!("{}:{:02}", t / 60, t % 60)
}

fn hit(x: i32, y: i32) -> Option<Btn> {
    PLAYER.with(|c| {
        let g = c.borrow();
        let p = g.as_ref()?;
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(p.hwnd, &mut rc);
        }
        let r = bar_rects(rc.right, rc.bottom, p.s, (90.0 * p.s) as i32);
        let inside = |q: &RECT| x >= q.left && x < q.right && y >= q.top && y < q.bottom;
        [(r.play, Btn::Play), (r.seek, Btn::Seek), (r.mute, Btn::Mute), (r.close, Btn::Close)]
            .into_iter()
            .find(|(q, _)| inside(q))
            .map(|(_, b)| b)
    })
}

fn paint_bar(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        let info = PLAYER.with(|c| {
            c.borrow().as_ref().map(|p| (p.s, p.pos, p.dur, p.paused || p.ended || !p.started, p.muted, p.gif, p.hover, p.font, p.font_b))
        });
        if let Some((s, pos, dur, show_play, muted, gif, hover, f, _fb)) = info {
            let b = bar_h(s);
            let (cw, ch) = (rc.right, rc.bottom);
            // 더블 버퍼(조작 줄만)
            let mem = CreateCompatibleDC(Some(hdc));
            let bmp = CreateCompatibleBitmap(hdc, cw, b);
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            let bg = CreateSolidBrush(rgb(0, 0, 0));
            FillRect(mem, &RECT { left: 0, top: 0, right: cw, bottom: b }, bg);
            let _ = DeleteObject(HGDIOBJ(bg.0));
            let r = bar_rects(cw, b, s, (90.0 * s) as i32);
            let white = rgb(0xe7, 0xe9, 0xea);
            let hot_bg = |q: &RECT, on: bool| {
                if on {
                    let br = CreateSolidBrush(rgb(0x2a, 0x2d, 0x30));
                    let oldb = SelectObject(mem, HGDIOBJ(br.0));
                    let oldp = SelectObject(mem, GetStockObject(NULL_PEN));
                    let pad = (6.0 * s) as i32;
                    let _ = RoundRect(mem, q.left + pad, q.top + pad, q.right - pad, q.bottom - pad, b, b);
                    SelectObject(mem, oldp);
                    SelectObject(mem, oldb);
                    let _ = DeleteObject(HGDIOBJ(br.0));
                }
            };
            let fill_poly = |pts: &[POINT], col: COLORREF| {
                let br = CreateSolidBrush(col);
                let oldb = SelectObject(mem, HGDIOBJ(br.0));
                let oldp = SelectObject(mem, GetStockObject(NULL_PEN));
                let _ = Polygon(mem, pts);
                SelectObject(mem, oldp);
                SelectObject(mem, oldb);
                let _ = DeleteObject(HGDIOBJ(br.0));
            };
            let u = |v: f32| (v * s).round() as i32;
            // 재생 · 멈춤
            hot_bg(&r.play, hover == Some(Btn::Play));
            let (cx, cy) = ((r.play.left + r.play.right) / 2, (r.play.top + r.play.bottom) / 2);
            if show_play {
                fill_poly(&[POINT { x: cx - u(5.0), y: cy - u(7.0) }, POINT { x: cx - u(5.0), y: cy + u(7.0) }, POINT { x: cx + u(7.0), y: cy }], white);
            } else {
                for dx in [-u(5.0), u(2.0)] {
                    fill_poly(&[POINT { x: cx + dx, y: cy - u(7.0) }, POINT { x: cx + dx + u(3.0), y: cy - u(7.0) }, POINT { x: cx + dx + u(3.0), y: cy + u(7.0) }, POINT { x: cx + dx, y: cy + u(7.0) }], white);
                }
            }
            // 진행 막대
            let ty = (r.seek.top + r.seek.bottom) / 2;
            let th = if hover == Some(Btn::Seek) { u(6.0) } else { u(4.0) };
            let track = CreateSolidBrush(rgb(0x3e, 0x41, 0x44));
            FillRect(mem, &RECT { left: r.seek.left, top: ty - th / 2, right: r.seek.right, bottom: ty + th / 2 }, track);
            let _ = DeleteObject(HGDIOBJ(track.0));
            let k = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) } else { 0.0 };
            let fx = r.seek.left + ((r.seek.right - r.seek.left) as f64 * k) as i32;
            let done = CreateSolidBrush(X_BLUE);
            FillRect(mem, &RECT { left: r.seek.left, top: ty - th / 2, right: fx, bottom: ty + th / 2 }, done);
            let oldb = SelectObject(mem, HGDIOBJ(done.0));
            let oldp = SelectObject(mem, GetStockObject(NULL_PEN));
            let rr = u(6.0);
            let _ = Ellipse(mem, fx - rr, ty - rr, fx + rr, ty + rr);
            SelectObject(mem, oldp);
            SelectObject(mem, oldb);
            let _ = DeleteObject(HGDIOBJ(done.0));
            // 시각
            SetBkMode(mem, TRANSPARENT);
            SetTextColor(mem, white);
            let oldf = SelectObject(mem, HGDIOBJ(f.0));
            let label = if gif { "GIF".to_string() } else { format!("{} / {}", fmt_t(pos), fmt_t(dur)) };
            let mut t: Vec<u16> = label.encode_utf16().collect();
            let mut tr = r.time;
            DrawTextW(mem, &mut t, &mut tr, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
            // 소리
            hot_bg(&r.mute, hover == Some(Btn::Mute));
            let (mx, my) = ((r.mute.left + r.mute.right) / 2 - u(3.0), (r.mute.top + r.mute.bottom) / 2);
            fill_poly(
                &[
                    POINT { x: mx - u(6.0), y: my - u(3.0) },
                    POINT { x: mx - u(2.0), y: my - u(3.0) },
                    POINT { x: mx + u(3.0), y: my - u(7.0) },
                    POINT { x: mx + u(3.0), y: my + u(7.0) },
                    POINT { x: mx - u(2.0), y: my + u(3.0) },
                    POINT { x: mx - u(6.0), y: my + u(3.0) },
                ],
                white,
            );
            let pen = CreatePen(PS_SOLID, u(1.6).max(1), white);
            let oldpen = SelectObject(mem, HGDIOBJ(pen.0));
            if muted {
                let _ = MoveToEx(mem, mx + u(6.0), my - u(4.0), None);
                let _ = LineTo(mem, mx + u(13.0), my + u(4.0));
                let _ = MoveToEx(mem, mx + u(13.0), my - u(4.0), None);
                let _ = LineTo(mem, mx + u(6.0), my + u(4.0));
            } else {
                let _ = MoveToEx(mem, mx + u(6.0), my - u(3.0), None);
                let _ = LineTo(mem, mx + u(8.0), my);
                let _ = LineTo(mem, mx + u(6.0), my + u(3.0));
                let _ = MoveToEx(mem, mx + u(9.0), my - u(6.0), None);
                let _ = LineTo(mem, mx + u(12.0), my);
                let _ = LineTo(mem, mx + u(9.0), my + u(6.0));
            }
            // 닫기 ✕
            hot_bg(&r.close, hover == Some(Btn::Close));
            let (qx, qy) = ((r.close.left + r.close.right) / 2, (r.close.top + r.close.bottom) / 2);
            let _ = MoveToEx(mem, qx - u(6.0), qy - u(6.0), None);
            let _ = LineTo(mem, qx + u(6.0), qy + u(6.0));
            let _ = MoveToEx(mem, qx + u(6.0), qy - u(6.0), None);
            let _ = LineTo(mem, qx - u(6.0), qy + u(6.0));
            SelectObject(mem, oldpen);
            let _ = DeleteObject(HGDIOBJ(pen.0));
            SelectObject(mem, oldf);
            let _ = BitBlt(hdc, 0, ch - b, cw, b, Some(mem), 0, 0, SRCCOPY);
            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(mem);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

/// 영상 자리: 받는 중 · 실패 · (재생 전 · 끝나면) 가운데 ▶.
fn paint_video(hwnd: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut ps);
        let info = PLAYER.with(|c| c.borrow().as_ref().map(|p| (p.status.clone(), p.mf.is_some(), p.started, p.ended, p.font_b, p.s)));
        if let Some((status, has_mf, started, ended, f, s)) = info {
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            if !has_mf || !started || ended {
                let bg = CreateSolidBrush(rgb(0, 0, 0));
                if !has_mf || !started {
                    FillRect(hdc, &rc, bg);
                }
                let _ = DeleteObject(HGDIOBJ(bg.0));
                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, rgb(0xe7, 0xe9, 0xea));
                let oldf = SelectObject(hdc, HGDIOBJ(f.0));
                let text = match &status {
                    Status::Loading => "영상 불러오는 중…".to_string(),
                    Status::Failed(m) => format!("{m}\n누르면 X 에서 열어요"),
                    Status::Ready if !started => "영상 불러오는 중…".to_string(),
                    Status::Ready => String::new(),
                };
                if !text.is_empty() {
                    let mut t: Vec<u16> = text.encode_utf16().collect();
                    let mut tr = rc;
                    let mut calc = rc;
                    DrawTextW(hdc, &mut t, &mut calc, DT_CENTER | DT_WORDBREAK | DT_NOPREFIX | windows::Win32::Graphics::Gdi::DT_CALCRECT);
                    let th = calc.bottom - calc.top;
                    tr.top = (rc.bottom - th) / 2;
                    DrawTextW(hdc, &mut t, &mut tr, DT_CENTER | DT_WORDBREAK | DT_NOPREFIX);
                } else if ended {
                    // 다시 보기 ▶ (X 파란 동그라미)
                    let (cx, cy) = (rc.right / 2, rc.bottom / 2);
                    let r = (28.0 * s) as i32;
                    let br = CreateSolidBrush(X_BLUE);
                    let oldb = SelectObject(hdc, HGDIOBJ(br.0));
                    let oldp = SelectObject(hdc, GetStockObject(NULL_PEN));
                    let _ = Ellipse(hdc, cx - r, cy - r, cx + r, cy + r);
                    let w = CreateSolidBrush(rgb(255, 255, 255));
                    SelectObject(hdc, HGDIOBJ(w.0));
                    let u = |v: f32| (v * s) as i32;
                    let _ = Polygon(hdc, &[POINT { x: cx - u(7.0), y: cy - u(11.0) }, POINT { x: cx - u(7.0), y: cy + u(11.0) }, POINT { x: cx + u(11.0), y: cy }]);
                    SelectObject(hdc, oldp);
                    SelectObject(hdc, oldb);
                    let _ = DeleteObject(HGDIOBJ(br.0));
                    let _ = DeleteObject(HGDIOBJ(w.0));
                }
                SelectObject(hdc, oldf);
                let _ = DT_LEFT;
            }
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

unsafe extern "system" fn video_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                paint_video(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_LBUTTONUP => {
                toggle_play();
                poll();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn lp_xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xFFFF) as u16 as i16 as i32, ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32)
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                paint_bar(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                layout(hwnd);
                LRESULT(0)
            }
            WM_TIMER => {
                poll();
                LRESULT(0)
            }
            m if m == net::WM_APP_NET => {
                poll();
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let (x, y) = lp_xy(lp);
                let h = hit(x, y);
                let changed = PLAYER.with(|c| {
                    let mut g = c.borrow_mut();
                    let p = g.as_mut()?;
                    if p.hover != h {
                        p.hover = h;
                        Some(())
                    } else {
                        None
                    }
                });
                if changed.is_some() {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_SETCURSOR => {
                let hot = PLAYER.with(|c| c.borrow().as_ref().is_some_and(|p| p.hover.is_some()));
                if let Ok(cur) = LoadCursorW(None, if hot { IDC_HAND } else { IDC_ARROW }) {
                    SetCursor(Some(cur));
                }
                LRESULT(1)
            }
            WM_LBUTTONUP => {
                let (x, y) = lp_xy(lp);
                match hit(x, y) {
                    Some(Btn::Play) => toggle_play(),
                    Some(Btn::Mute) => toggle_mute(),
                    Some(Btn::Close) => {
                        close();
                        return LRESULT(0);
                    }
                    Some(Btn::Seek) => {
                        let target = PLAYER.with(|c| {
                            c.borrow().as_ref().map(|p| {
                                let mut rc = RECT::default();
                                let _ = GetClientRect(p.hwnd, &mut rc);
                                let r = bar_rects(rc.right, rc.bottom, p.s, (90.0 * p.s) as i32);
                                let k = ((x - r.seek.left) as f64 / (r.seek.right - r.seek.left).max(1) as f64).clamp(0.0, 1.0);
                                k * p.dur
                            })
                        });
                        if let Some(t) = target {
                            seek_to(t);
                        }
                    }
                    None => {}
                }
                poll();
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let k = wp.0 as u16;
                if k == VK_ESCAPE.0 {
                    close();
                } else if k == VK_SPACE.0 {
                    toggle_play();
                } else if k == b'M' as u16 {
                    toggle_mute();
                } else if k == VK_LEFT.0 || k == VK_RIGHT.0 {
                    let pos = PLAYER.with(|c| c.borrow().as_ref().map(|p| p.pos)).unwrap_or(0.0);
                    seek_to(pos + if k == VK_LEFT.0 { -5.0 } else { 5.0 });
                }
                poll();
                LRESULT(0)
            }
            WM_ACTIVATE => {
                // 말풍선 · 영상 창 밖을 누르면 둘 다 닫는다.
                if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
                    let to = HWND(lp.0 as *mut c_void);
                    let owner = PLAYER.with(|c| c.borrow().as_ref().map(|p| p.req.owner));
                    if owner != Some(to) && !to.is_invalid() {
                        close();
                        crate::panel::close();
                    } else if to.is_invalid() {
                        close();
                        crate::panel::close();
                    }
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), TIMER_POLL);
                net::unregister(hwnd);
                let taken = PLAYER.with(|c| {
                    let mut g = c.borrow_mut();
                    if g.as_ref().is_some_and(|p| p.hwnd == hwnd) { g.take() } else { None }
                });
                if let Some(p) = taken {
                    if let Some(mf) = &p.mf {
                        let _ = mf.Shutdown();
                    }
                    let _ = DeleteObject(HGDIOBJ(p.font.0));
                    let _ = DeleteObject(HGDIOBJ(p.font_b.0));
                    if IsWindow(Some(p.req.owner)).as_bool() {
                        let _ = SetForegroundWindow(p.req.owner);
                    }
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MSG, PM_REMOVE, PeekMessageW};

    /// 실제 트윗 영상을 6초 틀고 상태를 찍는다(화면 아래쪽에 영상 창이 잠깐 뜬다 — 초점을 가져간다).
    /// 실행: cargo test play_video -- --ignored --nocapture
    #[test]
    #[ignore]
    fn play_video() {
        let work = work_area();
        let near = RECT { left: work.left + 200, top: work.bottom - 700, right: work.left + 700, bottom: work.bottom - 200 };
        play(Req {
            tweet_id: "2120851005571264560".into(),
            quote: true,
            thumb: "https://pbs.twimg.com/amplify_video_thumb/2120849014233137152/img/6cK1PDwIXc3TM3cu.jpg".into(),
            idx: 0,
            url: "https://x.com/i/status/2120851005571264560".into(),
            owner: HWND::default(),
            near,
        });
        let start = Instant::now();
        let mut msg = MSG::default();
        let mut last = String::new();
        while start.elapsed() < Duration::from_secs(8) {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    DispatchMessageW(&msg);
                }
            }
            let st = PLAYER.with(|c| {
                c.borrow().as_ref().map(|p| format!("{:?} mf={} started={} pos={:.1} dur={:.1}", p.status, p.mf.is_some(), p.started, p.pos, p.dur))
            });
            let st = st.unwrap_or_else(|| "closed".into());
            if st != last {
                println!("{:.1}s {st}", start.elapsed().as_secs_f32());
                last = st;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let ok = PLAYER.with(|c| c.borrow().as_ref().is_some_and(|p| p.started && p.pos > 0.5));
        close();
        assert!(ok, "재생이 시작돼 위치가 움직여야 한다");
    }
}
