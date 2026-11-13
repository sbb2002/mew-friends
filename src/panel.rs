//! 멤버 트윗 말풍선 — 캐릭터 바로 위에 뜨는 트윗 시트. 명세: docs/IMPL_SPEC.md §8.
//!
//! 생김새 · 동작은 mewtype-scheduler 의 트윗 시트(js/tweets.js · css/tweets.css `.tws`)를 따른다:
//! 멤버 색 그라데이션 바탕 · 머리 줄 멤버 칩 + 「번역 · 원문」 탭 · 날짜 구분선 · 글타래 · 글마다 두 면(번역 · X 원문 카드)을
//! 옆으로 밀어 넘기기 · 사진 · 영상(눌러서 재생) · 인용 · 위아래 끝 흐림 · ↓ 동그라미.
//! 열면 맨 아래(최신). 꼬리 끝이 캐릭터 머리를 가리킨다. 창 = 레이어드 팝업 하나(UpdateLayeredWindow).

mod card;
mod data;
mod sheet;
mod view;

pub use data::*;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::Once;
use std::time::{Duration, Instant};

use tiny_skia::{Pixmap, PixmapPaint, Transform};
use windows::Win32::Foundation::{COLORREF, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ,
    ReleaseDC, SelectObject,
};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_DOWN, VK_END,
    VK_ESCAPE, VK_HOME, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RIGHT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_BOTTOM, HWND_NOTOPMOST, HWND_TOP,
    HWND_TOPMOST, IDC_ARROW, IDC_HAND, IDC_SIZEWE, IsWindow, KillTimer, LoadCursorW,
    RegisterClassExW, SPI_GETWORKAREA, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SetCursor, SetForegroundWindow, SetTimer, SetWindowPos, SystemParametersInfoW, ULW_ALPHA,
    UpdateLayeredWindow, WA_INACTIVE, WINDOW_EX_STYLE, WM_ACTIVATE, WM_CAPTURECHANGED,
    WM_DESTROY, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_SETCURSOR, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

use crate::{net, player};

use self::card::{Act, Ctx};
use self::sheet::{CardVis, Layout, Vis};
use self::view::{Avatar, Fonts, Frame, Measurer, Palette, R, TextPainter};

/// 말풍선 최대 높이(CSS px).
const MAX_BUBBLE_H: f32 = 600.0;

/// 캐릭터 머리 위 한 점(화면 좌표) — 꼬리 끝이 여기를 가리킨다.
pub type Anchor = (i32, i32);

#[derive(Clone, Copy, PartialEq)]
enum Anim {
    Opening(Instant),
    Open,
    Closing(Instant),
}

const OPEN_MS: f32 = 320.0;
const CLOSE_MS: f32 = 210.0;
/// 면 넘기기(--t-face) · 제자리로 튕김(.is-spring)
const FACE_MS: f32 = 260.0;
const SPRING_MS: f32 = 420.0;
const TIMER_ANIM: usize = 1;
const TIMER_COPY: usize = 2;
const TIMER_AUTO: usize = 4;
/// 도넛 타이머 갱신(30초)
const TIMER_CLOCK: usize = 5;
/// 자동 말풍선이 떠 있는 시간(마우스가 위에 있는 동안은 멈추고, 나가면 처음부터).
const AUTO_MS: u32 = 5000;
/// WM_MOUSELEAVE · WM_MOUSEHWHEEL (feature 를 늘리지 않으려고 값을 직접 쓴다)
const WM_MOUSELEAVE: u32 = 0x02A3;
const WM_MOUSEHWHEEL: u32 = 0x020E;

/// 말풍선 종류. 자동 = 새 트윗으로 스스로 뜬 것(초점을 빼앗지 않고 5초 뒤 닫힘). 누르면 고정으로 바뀐다.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    Pinned,
    Auto,
}

struct Res {
    fonts: Fonts,
    me: Measurer,
    painter: TextPainter,
    avatars: Vec<Avatar>,
}

/// 면 넘기기 움직임.
#[derive(Clone, Copy)]
struct CardAnim {
    from: CardVis,
    to: CardVis,
    start: Instant,
    spring: bool,
    /// 끝나면 이 면이 앞
    end: u8,
}

/// 누른 채 끌기(카드 면 밀기).
#[derive(Clone)]
struct Drag {
    x0: f32,
    y0: f32,
    card: Option<String>,
    on: bool,
    dx: f32,
}

type BgKey = (i32, i32, u32, u32, String);

struct Bubble {
    hwnd: HWND,
    s: f32,
    data: PanelData,
    pal: Palette,
    lay: Layout,
    fr: Frame,
    scroll: f32,
    /// 부드러운 스크롤 목표
    target: f32,
    hover: Option<Act>,
    copied: Option<(String, bool)>,
    hits: Vec<(R, Act)>,
    img: Option<Pixmap>,
    pos: (i32, i32),
    anchor: Anchor,
    anim: Anim,
    mode: Mode,
    bg: Option<(BgKey, Pixmap)>,
    /// 탭 면(0 = 번역, 1 = 원문)
    face: u8,
    seg: f32,
    seg_anim: Option<(f32, f32, Instant)>,
    cards: HashMap<String, CardVis>,
    card_anims: HashMap<String, CardAnim>,
    faces: HashMap<(String, u8), (String, Pixmap)>,
    drag: Option<Drag>,
    /// 가로 휠 모으기(값, 마지막 시각, 쉬는 끝)
    hwheel: (f32, Instant, Instant),
}

thread_local! {
    static RES: RefCell<Option<Res>> = const { RefCell::new(None) };
    static BUBBLE: RefCell<Option<Bubble>> = const { RefCell::new(None) };
    static REQUESTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// 자동 말풍선을 눌러 고정한 멤버(메인 루프가 새 트윗 점을 지운다).
    static PINNED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// 항상 위(트레이 설정).
    static TOPMOST: Cell<bool> = const { Cell::new(false) };
    /// 탭 면 저장값(원본 mew:tllang) · 바뀌었음
    static FACE_PREF: Cell<u8> = const { Cell::new(0) };
    static FACE_CHANGED: Cell<bool> = const { Cell::new(false) };
}

fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None) }.map(|m| HINSTANCE(m.0)).unwrap_or_default()
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("MewFriendsBubble"),
            ..Default::default()
        };
        if RegisterClassExW(&wc) == 0 {
            eprintln!("[panel] 창 클래스 등록 실패");
        }
    });
}

fn work_area() -> RECT {
    let mut r = RECT { left: 0, top: 0, right: 1920, bottom: 1040 };
    unsafe {
        let _ = SystemParametersInfoW(SPI_GETWORKAREA, 0, Some(&mut r as *mut RECT as *mut c_void), Default::default());
    }
    r
}

fn scale() -> f32 {
    (unsafe { GetDpiForSystem() } as f32 / 96.0).max(0.5)
}

fn with_res<T>(s: f32, f: impl FnOnce(&mut Res) -> T) -> T {
    RES.with(|c| {
        let mut b = c.borrow_mut();
        let res = b.get_or_insert_with(|| Res {
            fonts: Fonts::new(s),
            me: Measurer::new(),
            painter: TextPainter::new(),
            avatars: ["arale", "yuno", "nonoka", "ritsu", "miyako"].iter().filter_map(|k| view::load_avatar(k)).collect(),
        });
        f(res)
    })
}

fn palette_for(member: &str, s: f32) -> Palette {
    with_res(s, |r| Palette::new(r.avatars.iter().find(|a| a.key == member).and_then(|a| a.color)))
}

fn layout_for(data: &PanelData, s: f32, pal: &Palette) -> Layout {
    with_res(s, |r| {
        let c = Ctx { s, me: &r.me, fonts: &r.fonts, pal, avatars: &r.avatars };
        Layout::build(data, &c)
    })
}

/// 말풍선 크기 · 위치를 앵커에 맞춘다(배치는 그대로).
fn geometry(lay: &Layout, s: f32, anchor: Anchor) -> (Frame, (i32, i32)) {
    let work = work_area();
    let sh = view::SHADOW * s;
    let tail = view::TAIL_H * s;
    let room = (anchor.1 - work.top) as f32 - sh - tail - 4.0;
    let max_h = (MAX_BUBBLE_H * s).min(room).max(lay.feed_top + 80.0 * s + lay.foot_h);
    let bh = (lay.feed_top + lay.content_h + lay.foot_h).min(max_h).round();
    let bw = view::BUBBLE_W * s;
    let win_w = bw + 2.0 * sh;
    let left = (anchor.0 as f32 - win_w / 2.0)
        .min(work.right as f32 - win_w + sh * 0.5)
        .max(work.left as f32 - sh * 0.5)
        .round();
    let edge = view::RADIUS * s + view::TAIL_HALF * s + 6.0 * s;
    let tail_x = (anchor.0 as f32 - (left + sh)).clamp(edge, bw - edge);
    let fr = Frame::new(s, bh, tail_x);
    let (_, tip_y) = fr.tail_tip(s);
    (fr, (left as i32, (anchor.1 as f32 - tip_y).round() as i32))
}

/// cubic-bezier(x1, y1, x2, y2) — x(u) 를 이분법으로 풀어 y(u).
fn bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let b = |u: f32, a: f32, c: f32| 3.0 * (1.0 - u).powi(2) * u * a + 3.0 * (1.0 - u) * u * u * c + u * u * u;
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if b(mid, x1, x2) < t { lo = mid } else { hi = mid }
    }
    b((lo + hi) / 2.0, y1, y2)
}

/// 원본 --ease = cubic-bezier(0.2, 0.8, 0.2, 1)
fn ease_out(t: f32) -> f32 {
    bezier(0.2, 0.8, 0.2, 1.0, t)
}

/// 끄는 동안의 두 면 모양(tweets.js ctl.move).
fn drag_vis(face: u8, dx: f32, w: f32) -> CardVis {
    let t = dx / w.max(1.0);
    let want = if face == 0 { t < 0.0 } else { t > 0.0 };
    let k = t.abs().min(1.0);
    let mut cv = CardVis::rest(face);
    cv.active = true;
    let back = 1 - face;
    if want {
        cv.vis[face as usize] = Vis { tx: t * w, op: (1.0 - k * 1.2).max(0.0), sc: 1.0 };
        cv.vis[back as usize] = Vis { tx: 0.0, op: (k * 1.6).min(1.0), sc: 0.985 + 0.015 * k };
        cv.dots = if face == 0 { k } else { 1.0 - k };
    } else {
        // 더 갈 면이 없는 쪽 = 고무줄
        cv.vis[face as usize].tx = dx.signum() * (dx.abs() / 3.0).min(24.0);
    }
    cv
}

/// 놓았을 때 갈 면(tweetlogic swipeTo — 25% 넘게 밀면 넘긴다).
pub fn swipe_to(face: u8, t: f32) -> u8 {
    let want = if face == 0 { t < 0.0 } else { t > 0.0 };
    if want && t.abs() > 0.25 { 1 - face } else { face }
}

impl Bubble {
    fn max_scroll(&self) -> f32 {
        sheet::max_scroll(&self.lay, &self.fr)
    }

    fn vis_of(&self, id: &str) -> CardVis {
        self.cards.get(id).copied().unwrap_or_else(|| CardVis::rest(self.face))
    }

    fn relayout(&mut self) {
        let at_bottom = self.scroll >= self.max_scroll() - 1.0;
        self.lay = layout_for(&self.data, self.s, &self.pal);
        let (fr, pos) = geometry(&self.lay, self.s, self.anchor);
        self.fr = fr;
        self.pos = pos;
        let max = self.max_scroll();
        self.scroll = if at_bottom { max } else { self.scroll.min(max) };
        self.target = if at_bottom { max } else { self.target.min(max) };
    }

    /// 다시 그린다(그림만). present 는 따로.
    fn redraw(&mut self) {
        net::drain();
        // 바탕은 크기 · 꼬리 · 색이 바뀔 때만
        let key = (self.fr.w, self.fr.h, self.fr.tail_x.to_bits(), self.fr.bh.to_bits(), self.pal.key());
        if self.bg.as_ref().is_none_or(|(k, _)| *k != key) {
            self.bg = view::background(&self.fr, &self.pal, self.s).map(|pm| (key, pm));
        }
        let s = self.s;
        let pal = self.pal;
        // 보이는 카드(+ 위아래 한 화면)의 면 그림을 준비한다. 서명이 같으면 다시 그리지 않는다.
        let view_h = self.fr.bh - self.lay.feed_top;
        let near: Vec<String> = sheet::visible_cards(&self.lay, &self.fr, self.scroll)
            .iter()
            .map(|c| c.tw.id.clone())
            .collect();
        let keep_lo = self.scroll - view_h;
        let keep_hi = self.scroll + 2.0 * view_h;
        let keep: Vec<String> = self
            .lay
            .cards()
            .filter(|c| c.y + c.h(s) > keep_lo && c.y < keep_hi)
            .map(|c| c.tw.id.clone())
            .collect();
        self.faces.retain(|(id, _), _| keep.contains(id));
        let hover = self.hover.clone();
        let copied = self.copied.clone();
        for id in &near {
            let cv = self.vis_of(id);
            let Some(card) = self.lay.card(id) else { continue };
            for f in sheet::needed_faces(&cv) {
                if f == 1 {
                    let _ = net::fx(id);
                }
                let sig = card::face_sig(card, f, hover.as_ref(), copied.as_ref().map(|(i, ok)| (i.as_str(), *ok)), &pal);
                let k = (id.clone(), f);
                if self.faces.get(&k).is_some_and(|(sg, _)| *sg == sig) {
                    continue;
                }
                let pm = with_res(s, |r| {
                    let c = Ctx { s, me: &r.me, fonts: &r.fonts, pal: &pal, avatars: &r.avatars };
                    card::render_face(&c, card, f, hover.as_ref(), copied.as_ref().map(|(i, ok)| (i.as_str(), *ok)), &mut r.painter)
                });
                if let Some(pm) = pm {
                    self.faces.insert(k, (sig, pm));
                }
            }
        }
        let Some((_, bg)) = &self.bg else {
            eprintln!("[panel] 바탕 그리기 실패");
            return;
        };
        let st = sheet::ViewState {
            scroll: self.scroll,
            hover: self.hover.as_ref(),
            seg: self.seg,
            face: self.face,
            cards: &self.cards,
            faces: &self.faces,
            now_ms: chrono::Utc::now().timestamp_millis(),
        };
        let out = with_res(s, |r| {
            let c = Ctx { s, me: &r.me, fonts: &r.fonts, pal: &pal, avatars: &r.avatars };
            sheet::render(&self.data, &self.lay, &self.fr, &st, &c, bg, &mut r.painter)
        });
        match out {
            Some(o) => {
                self.img = Some(o.pm);
                self.hits = o.hits;
            }
            None => eprintln!("[panel] 그리기 실패"),
        }
    }

    /// 지금 그림을 창에 올린다(열기 · 닫기 움직임 포함). 끝났으면 false.
    fn present(&self) -> bool {
        let Some(img) = &self.img else { return true };
        let (p, alive) = match self.anim {
            Anim::Open => (1.0, true),
            Anim::Opening(t0) => (ease_out(t0.elapsed().as_secs_f32() * 1000.0 / OPEN_MS), true),
            Anim::Closing(t0) => {
                let t = t0.elapsed().as_secs_f32() * 1000.0 / CLOSE_MS;
                (1.0 - ease_out(t), t < 1.0)
            }
        };
        let frame;
        let pm: &Pixmap = if p >= 0.999 {
            img
        } else {
            // 꼬리 끝을 중심으로 0.8 → 1 · 투명도 0 → 1
            let Some(mut f) = Pixmap::new(img.width(), img.height()) else { return alive };
            let (tx, ty) = self.fr.tail_tip(self.s);
            let k = 0.8 + 0.2 * p;
            let tr = Transform::from_row(k, 0.0, 0.0, k, tx * (1.0 - k), ty * (1.0 - k));
            let paint = PixmapPaint { opacity: p.clamp(0.0, 1.0), quality: tiny_skia::FilterQuality::Bilinear, ..PixmapPaint::default() };
            f.draw_pixmap(0, 0, img.as_ref(), &paint, tr, None);
            frame = f;
            &frame
        };
        put_layered(self.hwnd, pm, self.pos);
        alive
    }

    /// 움직임 한 박자(스크롤 · 탭 알약 · 면 넘기기). 아직 움직이면 true.
    fn step(&mut self) -> bool {
        let mut busy = false;
        // 스크롤
        let d = self.target - self.scroll;
        if d.abs() >= 0.5 {
            self.scroll += d * 0.3;
            busy = true;
        } else {
            self.scroll = self.target;
        }
        // 탭 알약
        if let Some((a, b, t0)) = self.seg_anim {
            let t = t0.elapsed().as_secs_f32() * 1000.0 / FACE_MS;
            self.seg = a + (b - a) * ease_out(t);
            if t >= 1.0 {
                self.seg = b;
                self.seg_anim = None;
            } else {
                busy = true;
            }
        }
        // 면 넘기기
        let mut done = Vec::new();
        for (id, an) in &self.card_anims {
            let ms = if an.spring { SPRING_MS } else { FACE_MS };
            let t = an.start.elapsed().as_secs_f32() * 1000.0 / ms;
            if t >= 1.0 {
                done.push(id.clone());
                continue;
            }
            busy = true;
            let k = if an.spring { bezier(0.25, 1.4, 0.45, 1.0, t) } else { ease_out(t) };
            let ko = ease_out(t);
            let mut cv = an.to;
            for f in 0..2 {
                let (a, b) = (an.from.vis[f], an.to.vis[f]);
                cv.vis[f] = Vis { tx: a.tx + (b.tx - a.tx) * k, op: a.op + (b.op - a.op) * ko, sc: a.sc + (b.sc - a.sc) * ko };
            }
            cv.dots = an.from.dots + (an.to.dots - an.from.dots) * ko;
            cv.front = an.from.front;
            cv.active = true;
            self.cards.insert(id.clone(), cv);
        }
        for id in done {
            if let Some(an) = self.card_anims.remove(&id) {
                self.cards.insert(id, CardVis::rest(an.end));
            }
        }
        busy
    }

    /// 카드 하나의 면을 바꾼다(smooth = 움직임).
    fn set_card_face(&mut self, id: &str, to: u8, smooth: bool) {
        let cur = self.vis_of(id);
        let from_face = self.card_anims.get(id).map(|a| a.end).unwrap_or(cur.front);
        if from_face == to && !cur.active {
            return;
        }
        if !smooth {
            self.card_anims.remove(id);
            self.cards.insert(id.to_string(), CardVis::rest(to));
            return;
        }
        let w = self.lay.card(id).map(|c| c.w).unwrap_or(400.0);
        // 앞 면은 ∓40% 로 밀려 나가며 흐려지고, 새 면은 제자리 · 불투명
        let mut target = CardVis::rest(to);
        target.active = true;
        let old = cur.front;
        if old != to {
            target.vis[old as usize] = Vis { tx: if to == 1 { -0.4 * w } else { 0.4 * w }, op: 0.0, sc: 1.0 };
        }
        let mut from = cur;
        from.front = old;
        self.card_anims.insert(id.to_string(), CardAnim { from, to: target, start: Instant::now(), spring: false, end: to });
    }

    /// 놓았는데 문턱을 못 넘음 → 앞 면이 제자리로 튕겨 돌아온다.
    fn spring_back(&mut self, id: &str) {
        let cur = self.vis_of(id);
        let face = cur.front;
        let mut to = CardVis::rest(face);
        to.active = true;
        self.card_anims.insert(id.to_string(), CardAnim { from: cur, to, start: Instant::now(), spring: true, end: face });
    }

    fn busy(&self) -> bool {
        !self.card_anims.is_empty() || self.seg_anim.is_some() || (self.target - self.scroll).abs() >= 0.5
    }
}

/// 픽스맵(RGBA premultiplied) → DIB(BGRA) → UpdateLayeredWindow(위치 · 크기 함께).
fn put_layered(hwnd: HWND, pm: &Pixmap, pos: (i32, i32)) {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    unsafe {
        let screen = GetDC(None);
        let dc: HDC = CreateCompatibleDC(Some(screen));
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: 0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = null_mut();
        let bmp: Option<HBITMAP> = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok();
        if let Some(bmp) = bmp {
            let old = SelectObject(dc, HGDIOBJ(bmp.0));
            let dst = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
            for (s, d) in pm.data().chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
            }
            let size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let dst_pt = POINT { x: pos.0, y: pos.1 };
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            if let Err(e) = UpdateLayeredWindow(hwnd, Some(screen), Some(&dst_pt), Some(&size), Some(dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA) {
                eprintln!("[panel] UpdateLayeredWindow 실패: {e}");
            }
            SelectObject(dc, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
    }
}

fn alive_hwnd() -> Option<HWND> {
    BUBBLE.with(|c| c.borrow().as_ref().map(|b| b.hwnd)).filter(|h| unsafe { IsWindow(Some(*h)) }.as_bool())
}

fn with_bubble<T>(f: impl FnOnce(&mut Bubble) -> T) -> Option<T> {
    BUBBLE.with(|c| c.borrow_mut().as_mut().map(f))
}

/// 다시 그려 올리고, 움직임이 남았으면 박자 타이머를 건다.
fn refresh() {
    let r = with_bubble(|b| {
        b.redraw();
        b.present();
        (b.hwnd, b.busy())
    });
    if let Some((h, busy)) = r {
        if busy {
            unsafe {
                let _ = SetTimer(Some(h), TIMER_ANIM, 15, None);
            }
        }
    }
}

/// 지금 열려 있는(닫히는 중이 아닌) 말풍선의 멤버.
pub fn current() -> Option<String> {
    BUBBLE.with(|c| c.borrow().as_ref().filter(|b| !matches!(b.anim, Anim::Closing(_))).map(|b| b.data.member.clone()))
}

/// 시험용: 지금 그림과 창 위치.
#[cfg(test)]
pub fn snapshot() -> Option<(Pixmap, (i32, i32))> {
    BUBBLE.with(|c| c.borrow().as_ref().and_then(|b| b.img.clone().map(|i| (i, b.pos))))
}

/// 말풍선 안에서 다른 멤버 칩을 눌렀다(메인 루프가 그 멤버로 다시 연다).
pub fn take_requests() -> Vec<String> {
    REQUESTS.with(|r| r.borrow_mut().drain(..).collect())
}

/// 자동 말풍선을 눌러 고정한 멤버들(지난 호출 뒤).
pub fn take_pinned() -> Vec<String> {
    PINNED.with(|r| r.borrow_mut().drain(..).collect())
}

/// 탭 면 저장값(설정 `tweet_face_orig`)을 넣는다 — 시작할 때.
pub fn set_face_pref(orig: bool) {
    FACE_PREF.with(|f| f.set(orig as u8));
}

/// 탭을 눌러 면 저장값이 바뀌었으면 새 값(메인 루프가 설정에 저장).
pub fn take_face_change() -> Option<bool> {
    FACE_CHANGED.with(|c| c.replace(false)).then(|| FACE_PREF.with(|f| f.get()) == 1)
}

/// 항상 위 켜기 · 끄기 — 열린 말풍선에도 바로 적용한다.
pub fn set_topmost(on: bool) {
    TOPMOST.with(|t| t.set(on));
    let st = BUBBLE.with(|c| c.borrow().as_ref().map(|b| (b.hwnd, b.mode)));
    if let Some((hwnd, mode)) = st {
        if !on {
            unsafe {
                let _ = SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
        raise(hwnd, mode, false);
    }
}

/// z 순서 · 활성화. 고정 = 맨 앞 + 초점(바깥을 누르면 닫히게). 자동 = 초점을 빼앗지 않고,
/// 항상 위면 맨 위 · 아니면 캐릭터처럼 다른 창 뒤(방해하지 않기).
fn raise(hwnd: HWND, mode: Mode, activate: bool) {
    let top = TOPMOST.with(|t| t.get());
    unsafe {
        match mode {
            Mode::Pinned => {
                let after = if top { HWND_TOPMOST } else { HWND_TOP };
                let _ = SetWindowPos(hwnd, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
                if activate {
                    let _ = SetForegroundWindow(hwnd);
                }
            }
            Mode::Auto => {
                let after = if top { HWND_TOPMOST } else { HWND_BOTTOM };
                let _ = SetWindowPos(hwnd, Some(after), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
        }
    }
}

/// 자동 말풍선 → 고정(평소처럼 계속 떠 있고, 바깥을 누르면 닫힌다).
fn pin() {
    let hit = with_bubble(|b| {
        if b.mode != Mode::Auto || matches!(b.anim, Anim::Closing(_)) {
            return None;
        }
        b.mode = Mode::Pinned;
        Some((b.hwnd, b.data.member.clone()))
    })
    .flatten();
    if let Some((hwnd, member)) = hit {
        unsafe {
            let _ = KillTimer(Some(hwnd), TIMER_AUTO);
        }
        PINNED.with(|p| p.borrow_mut().push(member));
        raise(hwnd, Mode::Pinned, true);
    }
}

fn arm_auto_timer(hwnd: HWND) {
    unsafe {
        let _ = SetTimer(Some(hwnd), TIMER_AUTO, AUTO_MS, None);
    }
}

fn mode_now() -> Option<Mode> {
    BUBBLE.with(|c| c.borrow().as_ref().map(|b| b.mode))
}

/// 말풍선을 띄운다(캐릭터 · 트레이 · 칩으로 연 것 = 고정). 같은 멤버가 이미 열려 있으면 닫는다
/// (캐릭터를 다시 누르면 닫힘 — 자동으로 떠 있던 것이면 고정으로 바꾼다). 다른 멤버가 열려 있으면 내용만 바꾼다.
pub fn show(data: PanelData, anchor: Anchor) {
    if current().as_deref() == Some(data.member.as_str()) {
        if mode_now() == Some(Mode::Auto) {
            pin();
        } else {
            close();
        }
        return;
    }
    open(data, anchor, Mode::Pinned);
}

/// 새 트윗으로 저절로 띄운다 — 초점을 빼앗지 않고 5초 뒤 닫힌다(마우스가 위에 있으면 기다림).
/// 사용자가 열어 둔(고정) 말풍선이 있으면 건드리지 않는다(같은 멤버면 내용만 갱신).
pub fn show_auto(data: PanelData, anchor: Anchor) {
    let pinned_open = BUBBLE.with(|c| c.borrow().as_ref().is_some_and(|b| b.mode == Mode::Pinned && !matches!(b.anim, Anim::Closing(_))));
    if pinned_open {
        update(data);
        return;
    }
    open(data, anchor, Mode::Auto);
}

fn open(data: PanelData, anchor: Anchor, mode: Mode) {
    let s = scale();
    let pal = palette_for(&data.member, s);
    let lay = layout_for(&data, s, &pal);
    let face = FACE_PREF.with(|f| f.get());
    if let Some(hwnd) = alive_hwnd() {
        with_bubble(|b| {
            let (fr, pos) = geometry(&lay, s, anchor);
            b.data = data;
            b.lay = lay;
            b.pal = pal;
            b.fr = fr;
            b.pos = pos;
            b.anchor = anchor;
            b.scroll = b.max_scroll();
            b.target = b.scroll;
            b.hover = None;
            b.copied = None;
            b.mode = mode;
            b.face = face;
            b.seg = face as f32;
            b.cards.clear();
            b.card_anims.clear();
            b.faces.clear();
            if matches!(b.anim, Anim::Closing(_)) {
                b.anim = Anim::Opening(Instant::now());
            }
            b.redraw();
            b.present();
        });
        unsafe {
            let _ = SetTimer(Some(hwnd), TIMER_ANIM, 15, None);
            if mode == Mode::Pinned {
                let _ = KillTimer(Some(hwnd), TIMER_AUTO);
            }
        }
        raise(hwnd, mode, true);
        if mode == Mode::Auto {
            arm_auto_timer(hwnd);
        }
        return;
    }

    ensure_class();
    let hwnd = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_LAYERED.0 | WS_EX_TOOLWINDOW.0),
            w!("MewFriendsBubble"),
            w!("Mew Friends"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinstance()),
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[panel] 창 생성 실패: {e}");
                return;
            }
        }
    };
    net::register(hwnd);
    let (fr, pos) = geometry(&lay, s, anchor);
    let now = Instant::now();
    let mut b = Bubble {
        hwnd,
        s,
        data,
        pal,
        lay,
        fr,
        scroll: 0.0,
        target: 0.0,
        hover: None,
        copied: None,
        hits: Vec::new(),
        img: None,
        pos,
        anchor,
        anim: Anim::Opening(now),
        mode,
        bg: None,
        face,
        seg: face as f32,
        seg_anim: None,
        cards: HashMap::new(),
        card_anims: HashMap::new(),
        faces: HashMap::new(),
        drag: None,
        hwheel: (0.0, now, now),
    };
    b.scroll = b.max_scroll();
    b.target = b.scroll;
    b.redraw();
    b.present();
    BUBBLE.with(|c| *c.borrow_mut() = Some(b));
    raise(hwnd, mode, true);
    unsafe {
        let _ = SetTimer(Some(hwnd), TIMER_ANIM, 15, None);
        let _ = SetTimer(Some(hwnd), TIMER_CLOCK, 30_000, None);
    }
    if mode == Mode::Auto {
        arm_auto_timer(hwnd);
    }
}

/// 폴링으로 데이터가 바뀌었을 때 — 열린 말풍선의 내용만 갈아 끼운다(맨 아래에 있었으면 맨 아래 유지).
pub fn update(data: PanelData) {
    let changed = with_bubble(|b| {
        if b.data.member != data.member || b.data == data || matches!(b.anim, Anim::Closing(_)) {
            return false;
        }
        b.data = data;
        b.relayout();
        b.faces.clear();
        true
    })
    .unwrap_or(false);
    if changed {
        refresh();
    }
}

/// 캐릭터가 움직였으면 따라간다(같으면 아무것도 안 한다).
pub fn follow(anchor: Anchor) {
    with_bubble(|b| {
        if b.anchor == anchor {
            return;
        }
        b.anchor = anchor;
        let (fr, pos) = geometry(&b.lay, b.s, anchor);
        let reshaped = (fr.tail_x - b.fr.tail_x).abs() > 0.5 || (fr.bh - b.fr.bh).abs() > 0.5;
        b.pos = pos;
        if reshaped {
            b.fr = fr;
            b.scroll = b.scroll.min(b.max_scroll());
            b.target = b.scroll;
            b.redraw();
        }
        b.present();
    });
}

/// 닫는다(움직임 뒤 창을 없앤다).
pub fn close() {
    let hwnd = with_bubble(|b| {
        if matches!(b.anim, Anim::Closing(_)) {
            return None;
        }
        b.anim = Anim::Closing(Instant::now());
        Some(b.hwnd)
    })
    .flatten();
    if let Some(h) = hwnd {
        player::close();
        unsafe {
            let _ = SetTimer(Some(h), TIMER_ANIM, 15, None);
        }
    }
}

fn copy_text(hwnd: HWND, text: &str) -> bool {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let ok = (|| {
            let h = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2).ok()?;
            let p = GlobalLock(h) as *mut u16;
            if p.is_null() {
                return None;
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), p, wide.len());
            let _ = GlobalUnlock(h);
            // CF_UNICODETEXT = 13
            SetClipboardData(13, Some(HANDLE(h.0))).ok()?;
            Some(())
        })()
        .is_some();
        let _ = CloseClipboard();
        ok
    }
}

fn lparam_xy(lp: LPARAM) -> (f32, f32) {
    ((lp.0 & 0xFFFF) as u16 as i16 as f32, ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as f32)
}

/// 누를 곳(밀기 손잡이 Track 은 맨 아래 — 단추가 우선).
fn hit_at(x: f32, y: f32) -> Option<Act> {
    BUBBLE.with(|c| {
        let g = c.borrow();
        let b = g.as_ref()?;
        b.hits
            .iter()
            .rev()
            .filter(|(r, _)| r.contains(x, y))
            .map(|(_, a)| a.clone())
            .find(|a| !matches!(a, Act::Track(_)))
            .or_else(|| b.hits.iter().rev().find(|(r, a)| matches!(a, Act::Track(_)) && r.contains(x, y)).map(|(_, a)| a.clone()))
    })
}

fn set_hover(new: Option<Act>) {
    let new = new.filter(|a| !matches!(a, Act::Track(_)));
    let changed = with_bubble(|b| {
        if b.hover != new {
            b.hover = new;
            true
        } else {
            false
        }
    })
    .unwrap_or(false);
    if changed {
        refresh();
    }
}

fn scroll_to(target: f32) {
    with_bubble(|b| b.target = target.clamp(0.0, b.max_scroll()));
    refresh();
}

fn scroll_by(dy: f32) {
    let t = BUBBLE.with(|c| c.borrow().as_ref().map(|b| b.target + dy));
    if let Some(t) = t {
        scroll_to(t);
    }
}

/// 탭: 모든 글을 그 면으로(보이는 글은 움직임).
fn set_global_face(f: u8) {
    FACE_PREF.with(|p| p.set(f));
    FACE_CHANGED.with(|c| c.set(true));
    with_bubble(|b| {
        if b.face == f && b.seg_anim.is_none() {
            return;
        }
        b.face = f;
        b.seg_anim = Some((b.seg, f as f32, Instant::now()));
        let vis: Vec<String> = sheet::visible_cards(&b.lay, &b.fr, b.scroll).iter().map(|c| c.tw.id.clone()).collect();
        let all: Vec<String> = b.lay.cards().map(|c| c.tw.id.clone()).collect();
        for id in all {
            let smooth = vis.contains(&id);
            b.set_card_face(&id, f, smooth);
        }
    });
    refresh();
}

fn on_click(hwnd: HWND, act: Act) {
    match act {
        Act::Chip(k) => REQUESTS.with(|r| r.borrow_mut().push(k)),
        Act::Seg(f) => set_global_face(f),
        Act::Live => {
            let url = BUBBLE.with(|c| c.borrow().as_ref().and_then(|b| b.data.live.as_ref().map(|l| l.1.clone())));
            if let Some(u) = url {
                crate::util::open_url(&u);
            }
        }
        Act::Open(u) => crate::util::open_url(&u),
        Act::Copy(id) => {
            let url = BUBBLE.with(|c| {
                c.borrow().as_ref().and_then(|b| b.data.units.iter().flat_map(|u| &u.tweets).find(|t| t.id == id).and_then(|t| t.url.clone()))
            });
            let ok = url.is_some_and(|u| copy_text(hwnd, &u));
            with_bubble(|b| b.copied = Some((id, ok)));
            refresh();
            unsafe {
                let _ = SetTimer(Some(hwnd), TIMER_COPY, 1500, None);
            }
        }
        Act::Fab => {
            let max = BUBBLE.with(|c| c.borrow().as_ref().map(|b| b.max_scroll())).unwrap_or(0.0);
            scroll_to(max);
        }
        Act::Video { id, quote, thumb, idx, url } => {
            let near = BUBBLE.with(|c| {
                c.borrow().as_ref().map(|b| RECT {
                    left: b.pos.0,
                    top: b.pos.1,
                    right: b.pos.0 + b.fr.w,
                    bottom: b.pos.1 + b.fr.h,
                })
            });
            if let Some(near) = near {
                player::play(player::Req { tweet_id: id, quote, thumb, idx, url, owner: hwnd, near });
            }
        }
        Act::Track(_) => {}
    }
}

/// 누른 곳에서 끌기 시작(카드 위면 그 카드).
fn drag_start(hwnd: HWND, x: f32, y: f32) {
    let card = match hit_at(x, y) {
        Some(Act::Track(id)) => Some(id),
        Some(Act::Copy(_)) | Some(Act::Open(_)) | Some(Act::Video { .. }) | None => {
            // 단추 위에서 시작해도 카드 밀기는 된다(링크 a 위는 원본도 막지만 위젯은 손잡이가 넓은 편이 편하다)
            BUBBLE.with(|c| {
                c.borrow().as_ref().and_then(|b| {
                    b.hits.iter().find(|(r, a)| matches!(a, Act::Track(_)) && r.contains(x, y)).map(|(_, a)| match a {
                        Act::Track(id) => id.clone(),
                        _ => unreachable!(),
                    })
                })
            })
        }
        _ => None,
    };
    if let Some(id) = &card {
        let _ = net::fx(id);
    }
    with_bubble(|b| b.drag = Some(Drag { x0: x, y0: y, card, on: false, dx: 0.0 }));
    unsafe {
        SetCapture(hwnd);
    }
}

/// 끄는 중. 끌기가 시작됐으면 true.
fn drag_move(x: f32, y: f32) -> bool {
    let res = with_bubble(|b| {
        let mut d = b.drag.clone()?;
        let id = d.card.clone()?;
        let (dx, dy) = (x - d.x0, y - d.y0);
        if !d.on {
            if dx.abs() < 8.0 * b.s || dx.abs() < dy.abs() {
                return None;
            }
            d.on = true;
            b.card_anims.remove(&id);
            b.hover = None;
        }
        d.dx = dx;
        let face = b.vis_of(&id).front;
        let w = b.lay.card(&id).map(|c| c.w).unwrap_or(400.0);
        b.cards.insert(id, drag_vis(face, dx, w));
        b.drag = Some(d);
        Some(())
    })
    .flatten();
    if res.is_some() {
        refresh();
        true
    } else {
        false
    }
}

/// 놓음. 끌었으면 true(클릭으로 치지 않는다).
fn drag_end() -> bool {
    let was = with_bubble(|b| {
        let d = b.drag.take()?;
        if !d.on {
            return None;
        }
        let id = d.card?;
        let cv = b.vis_of(&id);
        let w = b.lay.card(&id).map(|c| c.w).unwrap_or(400.0);
        let to = swipe_to(cv.front, d.dx / w.max(1.0));
        if to == cv.front {
            b.spring_back(&id);
        } else {
            b.set_card_face(&id, to, true);
        }
        Some(())
    })
    .flatten()
    .is_some();
    unsafe {
        let _ = ReleaseCapture();
    }
    if was {
        refresh();
    }
    was
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                match wp.0 {
                    TIMER_ANIM => {
                        let (alive, done) = with_bubble(|b| {
                            let busy = b.step();
                            let mut opening = false;
                            if let Anim::Opening(t0) = b.anim {
                                if t0.elapsed() >= Duration::from_millis(OPEN_MS as u64) {
                                    b.anim = Anim::Open;
                                } else {
                                    opening = true;
                                }
                            }
                            if busy {
                                b.redraw();
                            }
                            let alive = b.present();
                            let closing = matches!(b.anim, Anim::Closing(_));
                            (alive, !busy && !opening && !closing)
                        })
                        .unwrap_or((false, true));
                        if done {
                            let _ = KillTimer(Some(hwnd), TIMER_ANIM);
                        }
                        if !alive {
                            let _ = DestroyWindow(hwnd);
                        }
                    }
                    TIMER_COPY => {
                        let _ = KillTimer(Some(hwnd), TIMER_COPY);
                        with_bubble(|b| b.copied = None);
                        refresh();
                    }
                    TIMER_CLOCK => {
                        if BUBBLE.with(|c| c.borrow().as_ref().is_some_and(|b| b.data.upcoming.is_some())) {
                            refresh();
                        }
                    }
                    TIMER_AUTO => {
                        let _ = KillTimer(Some(hwnd), TIMER_AUTO);
                        if mode_now() == Some(Mode::Auto) {
                            close();
                        }
                    }
                    _ => {}
                }
                LRESULT(0)
            }
            m if m == net::WM_APP_NET => {
                // 그림 · 원문 정보가 왔다 — 1칸 비율 · 원문 면 이름이 바뀔 수 있어 배치부터
                if net::drain() || true {
                    with_bubble(|b| b.relayout());
                    refresh();
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut tme);
                if mode_now() == Some(Mode::Auto) {
                    let _ = KillTimer(Some(hwnd), TIMER_AUTO);
                }
                let (x, y) = lparam_xy(lp);
                let dragging = BUBBLE.with(|c| c.borrow().as_ref().is_some_and(|b| b.drag.is_some()));
                if dragging {
                    drag_move(x, y);
                } else {
                    set_hover(hit_at(x, y));
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                if mode_now() == Some(Mode::Auto) {
                    arm_auto_timer(hwnd);
                }
                set_hover(None);
                LRESULT(0)
            }
            WM_SETCURSOR => {
                let (hot, grab) = BUBBLE.with(|c| {
                    c.borrow().as_ref().map(|b| (b.hover.is_some(), b.drag.as_ref().is_some_and(|d| d.on))).unwrap_or((false, false))
                });
                let cur = if grab { IDC_SIZEWE } else if hot { IDC_HAND } else { IDC_ARROW };
                if let Ok(cur) = LoadCursorW(None, cur) {
                    SetCursor(Some(cur));
                }
                LRESULT(1)
            }
            WM_LBUTTONDOWN => {
                // 자동 말풍선을 누르면 평소처럼 고정
                pin();
                let (x, y) = lparam_xy(lp);
                drag_start(hwnd, x, y);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let (x, y) = lparam_xy(lp);
                if !drag_end() {
                    if let Some(a) = hit_at(x, y) {
                        on_click(hwnd, a);
                    }
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                // 끄는 중 붙잡기를 잃으면 제자리로
                let lost = with_bubble(|b| {
                    let d = b.drag.take()?;
                    let id = d.card.filter(|_| d.on)?;
                    b.spring_back(&id);
                    Some(())
                })
                .flatten();
                if lost.is_some() {
                    refresh();
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
                let s = BUBBLE.with(|c| c.borrow().as_ref().map(|b| b.s)).unwrap_or(1.0);
                scroll_by(-delta / 120.0 * 72.0 * s);
                LRESULT(0)
            }
            WM_MOUSEHWHEEL => {
                // 트랙패드 가로: 60 넘게 모이면 한 번 넘기고 400ms 쉰다
                let delta = ((wp.0 >> 16) & 0xFFFF) as u16 as i16 as f32;
                let mut pt = POINT { x: (lp.0 & 0xFFFF) as u16 as i16 as i32, y: ((lp.0 >> 16) & 0xFFFF) as u16 as i16 as i32 };
                let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
                if let Some(Act::Track(id)) = BUBBLE.with(|c| {
                    c.borrow().as_ref().and_then(|b| {
                        b.hits.iter().find(|(r, a)| matches!(a, Act::Track(_)) && r.contains(pt.x as f32, pt.y as f32)).map(|(_, a)| a.clone())
                    })
                }) {
                    with_bubble(|b| {
                        let now = Instant::now();
                        if now < b.hwheel.2 {
                            return;
                        }
                        if now.duration_since(b.hwheel.1) > Duration::from_millis(250) {
                            b.hwheel.0 = 0.0;
                        }
                        b.hwheel.1 = now;
                        b.hwheel.0 += delta;
                        if b.hwheel.0.abs() > 60.0 {
                            let to = if b.hwheel.0 > 0.0 { 1 } else { 0 };
                            let _ = net::fx(&id);
                            b.set_card_face(&id, to, true);
                            b.hwheel = (0.0, now, now + Duration::from_millis(400));
                        }
                    });
                    refresh();
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let (s, max, view_h) = BUBBLE
                    .with(|c| c.borrow().as_ref().map(|b| (b.s, b.max_scroll(), b.fr.bh - b.lay.feed_top)))
                    .unwrap_or((1.0, 0.0, 300.0));
                let k = wp.0 as u16;
                if k == VK_ESCAPE.0 {
                    close();
                } else if k == VK_UP.0 {
                    scroll_by(-48.0 * s);
                } else if k == VK_DOWN.0 {
                    scroll_by(48.0 * s);
                } else if k == VK_PRIOR.0 {
                    scroll_by(-view_h * 0.85);
                } else if k == VK_NEXT.0 {
                    scroll_by(view_h * 0.85);
                } else if k == VK_HOME.0 {
                    scroll_to(0.0);
                } else if k == VK_END.0 {
                    scroll_to(max);
                } else if k == VK_LEFT.0 || k == VK_RIGHT.0 {
                    // ← → = 탭(모든 글) 넘기기
                    set_global_face(if k == VK_RIGHT.0 { 1 } else { 0 });
                }
                LRESULT(0)
            }
            WM_ACTIVATE => {
                // 다른 곳을 누르면 닫힌다(원본: 바깥 어둠을 누르면 닫힘). 영상 창으로 가는 것은 빼고.
                if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
                    let to = HWND(lp.0 as *mut c_void);
                    if mode_now() == Some(Mode::Pinned) && !player::is_player(to) {
                        close();
                    }
                } else {
                    pin();
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                net::unregister(hwnd);
                BUBBLE.with(|c| {
                    let mut g = c.borrow_mut();
                    if g.as_ref().is_some_and(|b| b.hwnd == hwnd) {
                        *g = None;
                    }
                });
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{parse_preview, parse_tweets};
    use crate::util::parse_utc;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GWL_EXSTYLE, GetWindowLongW, MSG, PM_REMOVE, PeekMessageW, PostMessageW,
        TranslateMessage, WS_EX_TOPMOST,
    };

    #[test]
    fn swipe_threshold() {
        assert_eq!(swipe_to(0, -0.3), 1);
        assert_eq!(swipe_to(0, -0.2), 0);
        assert_eq!(swipe_to(0, 0.5), 0); // 번역 면에서 오른쪽은 갈 곳 없음
        assert_eq!(swipe_to(1, 0.3), 0);
        let v = drag_vis(0, -100.0, 400.0);
        assert!(v.active && v.vis[0].tx == -100.0 && v.vis[1].op > 0.0);
        let r = drag_vis(0, 90.0, 400.0);
        assert_eq!(r.vis[0].tx, 24.0); // 고무줄 상한
    }

    fn pump(ms: u64) {
        let start = Instant::now();
        let mut msg = MSG::default();
        while start.elapsed() < Duration::from_millis(ms) {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn save(img: &Pixmap, name: &str) {
        let mut out = Pixmap::new(img.width(), img.height()).unwrap();
        out.fill(tiny_skia::Color::from_rgba8(200, 205, 210, 255));
        out.draw_pixmap(0, 0, img.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("target/{name}.png"));
        let _ = out.save_png(&path);
        println!("saved {}", path.display());
    }

    fn live_data() -> Option<(crate::model::Preview, crate::model::TweetsFile)> {
        let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
        let r = crate::poll::fetch_once(&agent);
        Some((r.preview?, r.tweets?))
    }

    /// 실제 데이터로 말풍선을 띄워 번역 면 · 원문 면 · 밀기 중간을 `target/bubble_*.png` 로 저장한다(그림 · 원문 정보를 받아 온다).
    /// 실행: cargo test shot_bubble -- --ignored --nocapture
    #[test]
    #[ignore]
    fn shot_bubble() {
        let (p, t) = live_data().unwrap_or_else(|| {
            (parse_preview(include_str!("../fixtures/preview.sample.json")).unwrap(), parse_tweets(include_str!("../fixtures/tweets.sample.json")).unwrap())
        });
        let now = chrono::Utc::now();
        // 미디어 · 인용이 있는 멤버를 고른다
        let member = p
            .order()
            .into_iter()
            .max_by_key(|m| {
                build_data(m, &p, &t, now).units.iter().flat_map(|u| &u.tweets).filter(|x| !x.media.is_empty() || x.quote.is_some()).count()
            })
            .unwrap();
        let work = work_area();
        set_face_pref(false);
        show(build_data(&member, &p, &t, now), (work.left + 500, work.bottom - 140));
        pump(6000); // 그림 받기
        // 미디어 있는 글이 보이게 스크롤
        let target = BUBBLE.with(|c| {
            let g = c.borrow();
            let b = g.as_ref().unwrap();
            b.lay.cards().filter(|c| !c.tw.media.is_empty() || c.tw.quote.is_some()).last().map(|c| c.y - 10.0)
        });
        if let Some(y) = target {
            with_bubble(|b| {
                b.scroll = y.clamp(0.0, b.max_scroll());
                b.target = b.scroll;
            });
        }
        refresh();
        pump(4000);
        refresh();
        save(&snapshot().unwrap().0, "bubble_ko");
        set_global_face(1);
        pump(4000);
        refresh();
        save(&snapshot().unwrap().0, "bubble_orig");
        // 밀기 중간(원문 → 번역 30%)
        let id = BUBBLE.with(|c| {
            let g = c.borrow();
            let b = g.as_ref().unwrap();
            sheet::visible_cards(&b.lay, &b.fr, b.scroll).first().map(|c| c.tw.id.clone())
        });
        if let Some(id) = id {
            with_bubble(|b| {
                let w = b.lay.card(&id).unwrap().w;
                b.cards.insert(id.clone(), drag_vis(1, 0.3 * w, w));
            });
            refresh();
            save(&snapshot().unwrap().0, "bubble_swipe");
        }
        close();
        pump(400);
    }

    /// 사진 있는 글의 번역 면(위) · 원문 면(아래 끝)을 `target/bubble_media_*.png` 로.
    /// 실행: cargo test --release shot_media -- --ignored --nocapture
    #[test]
    #[ignore]
    fn shot_media() {
        let (p, t) = live_data().expect("실데이터");
        let now = chrono::Utc::now();
        let (member, id) = p
            .order()
            .into_iter()
            .find_map(|m| {
                let d = build_data(&m, &p, &t, now);
                let id = d.units.iter().flat_map(|u| &u.tweets).filter(|x| x.media.len() >= 1).max_by_key(|x| x.media.len()).map(|x| x.id.clone());
                id.map(|i| (m, i))
            })
            .expect("사진 있는 글");
        let work = work_area();
        set_face_pref(false);
        show(build_data(&member, &p, &t, now), (work.left + 500, work.bottom - 140));
        let go = |bottom: bool| {
            with_bubble(|b| {
                let c = b.lay.card(&id).unwrap();
                let view_h = b.fr.bh - b.lay.feed_top;
                let y = if bottom { c.y + c.track_h + 30.0 - view_h } else { c.y - 8.0 };
                b.scroll = y.clamp(0.0, b.max_scroll());
                b.target = b.scroll;
            });
        };
        go(false);
        refresh();
        pump(6000);
        go(false);
        refresh();
        save(&snapshot().unwrap().0, "bubble_media_ko");
        with_bubble(|b| b.set_card_face(&id, 1, false));
        refresh();
        pump(4000);
        go(true);
        refresh();
        save(&snapshot().unwrap().0, "bubble_media_orig");
        close();
        pump(400);
    }

    /// 영상 칸을 누르면 영상 창이 떠서 재생되는지 — 화면을 `target/video_play.png` 로(영상 창이 몇 초 화면에 뜬다).
    /// 실행: cargo test --release video_from_bubble -- --ignored --nocapture
    #[test]
    #[ignore]
    fn video_from_bubble() {
        let (p, t) = live_data().expect("실데이터");
        let now = chrono::Utc::now();
        let (member, act) = p
            .order()
            .into_iter()
            .find_map(|m| {
                let d = build_data(&m, &p, &t, now);
                let tw = d.units.iter().flat_map(|u| &u.tweets).find(|x| {
                    x.media.iter().chain(x.quote.iter().flat_map(|q| q.media.iter())).any(|u| net::is_video_url(u))
                })?;
                let quote = !tw.media.iter().any(|u| net::is_video_url(u));
                let list = if quote { &tw.quote.as_ref()?.media } else { &tw.media };
                let thumb = list.iter().find(|u| net::is_video_url(u))?.clone();
                Some((m, Act::Video { id: tw.id.clone(), quote, thumb, idx: 0, url: tw.url.clone().unwrap_or_default() }))
            })
            .expect("영상 있는 글");
        let work = work_area();
        show(build_data(&member, &p, &t, now), (work.left + 500, work.bottom - 140));
        pump(500);
        let hwnd = alive_hwnd().unwrap();
        on_click(hwnd, act);
        pump(5000);
        let st = player::debug_state();
        println!("player: {st:?}");
        if let Some(rc) = player::rect() {
            unsafe {
                use windows::Win32::Graphics::Gdi::{BitBlt, CreateCompatibleBitmap, GetDIBits, SRCCOPY, BI_RGB};
                let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
                let screen = GetDC(None);
                let mem = CreateCompatibleDC(Some(screen));
                let bmp = CreateCompatibleBitmap(screen, w, h);
                let old = SelectObject(mem, HGDIOBJ(bmp.0));
                let _ = BitBlt(mem, 0, 0, w, h, Some(screen), rc.left, rc.top, SRCCOPY);
                let mut bi = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
                let mut buf = vec![0u8; (w * h * 4) as usize];
                GetDIBits(mem, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut c_void), &mut bi, DIB_RGB_COLORS);
                for px in buf.chunks_exact_mut(4) { px.swap(0, 2); px[3] = 255; }
                SelectObject(mem, old);
                let _ = DeleteObject(HGDIOBJ(bmp.0));
                let _ = DeleteDC(mem);
                ReleaseDC(None, screen);
                let img = image::RgbaImage::from_raw(w as u32, h as u32, buf).unwrap();
                img.save(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/video_play.png")).unwrap();
            }
        }
        player::close();
        close();
        pump(400);
        assert!(st.is_some_and(|(started, pos)| started && pos > 0.3), "재생이 시작돼야 한다");
    }

    /// 24시간 안 예고가 있는 멤버의 미니 예고 카드 — 실제 · 50분 남음 · 지각을 `target/bubble_mini_*.png` 로.
    /// 실행: cargo test --release shot_mini -- --ignored --nocapture
    #[test]
    #[ignore]
    fn shot_mini() {
        let (p, t) = live_data().expect("실데이터");
        let now = chrono::Utc::now();
        let (member, mut data) = p
            .order()
            .into_iter()
            .map(|m| (m.clone(), build_data(&m, &p, &t, now)))
            .find(|(_, d)| d.upcoming.is_some())
            .unwrap_or_else(|| {
                // 지금 24시간 안 예고가 없으면 유노에 시험 예고를 붙인다(메모리에서만)
                let mut d = build_data("yuno", &p, &t, now);
                d.upcoming = Some(Upcoming {
                    title: "시험 예고 — 노래 방송".into(),
                    start_ms: now.timestamp_millis() + 8 * 3_600_000,
                    url: "https://www.youtube.com/".into(),
                    thumb: None,
                    collab: true,
                    membership: false,
                });
                ("yuno".into(), d)
            });
        println!("member={member} upcoming={:?}", data.upcoming.as_ref().map(|u| (&u.title, when_label(u.start_ms))));
        let work = work_area();
        let anchor = (work.left + 500, work.bottom - 140);
        show(data.clone(), anchor);
        pump(4000);
        refresh();
        save(&snapshot().unwrap().0, "bubble_mini_real");
        let now_ms = now.timestamp_millis();
        for (name, start) in [("50min", now_ms + 50 * 60_000), ("late", now_ms - 10 * 60_000)] {
            if let Some(u) = data.upcoming.as_mut() {
                u.start_ms = start;
            }
            update(data.clone());
            pump(300);
            refresh();
            save(&snapshot().unwrap().0, &format!("bubble_mini_{name}"));
        }
        close();
        pump(400);
    }

    /// 자동 말풍선: 5초 뒤 닫힘 · 마우스가 오면 처음부터 · 누르면 고정 · 항상 위 설정.
    /// 실행: cargo test auto_bubble -- --ignored --nocapture (바탕화면 아래쪽에 잠깐 말풍선이 뜬다)
    #[test]
    #[ignore]
    fn auto_bubble() {
        let p = parse_preview(include_str!("../fixtures/preview.sample.json")).unwrap();
        let t = parse_tweets(include_str!("../fixtures/tweets.sample.json")).unwrap();
        let now = parse_utc("2026-09-07T12:30:00Z").unwrap();
        let work = work_area();
        let anchor = (work.left + 400, work.bottom - 140);
        let data = || build_data("yuno", &p, &t, now);
        let ex = |h: HWND| unsafe { GetWindowLongW(h, GWL_EXSTYLE) } as u32 & WS_EX_TOPMOST.0 != 0;
        let state = || BUBBLE.with(|c| c.borrow().as_ref().map(|b| (b.mode, matches!(b.anim, Anim::Closing(_)), b.hwnd)));

        set_topmost(false);
        show_auto(data(), anchor);
        pump(300);
        let (m, _, h) = state().unwrap();
        assert_eq!(m, Mode::Auto);
        assert!(!ex(h));
        pump(4000);
        assert!(state().is_some_and(|s| !s.1), "4.3초엔 아직 떠 있어야 한다");
        pump(1500);
        assert!(state().is_none_or(|s| s.1), "5초가 지나면 닫힌다");
        pump(400);

        show_auto(data(), anchor);
        pump(3000);
        let h = state().unwrap().2;
        unsafe {
            let _ = PostMessageW(Some(h), WM_MOUSEMOVE, WPARAM(0), LPARAM((40 << 16) | 40));
        }
        pump(3500);
        assert!(state().is_some_and(|s| !s.1 && s.0 == Mode::Auto), "마우스 뒤 다시 5초");
        pump(2200);
        assert!(state().is_none_or(|s| s.1));
        pump(400);

        show_auto(data(), anchor);
        pump(500);
        let h = state().unwrap().2;
        unsafe {
            let _ = PostMessageW(Some(h), WM_LBUTTONDOWN, WPARAM(1), LPARAM((40 << 16) | 40));
            let _ = PostMessageW(Some(h), WM_LBUTTONUP, WPARAM(0), LPARAM((40 << 16) | 40));
        }
        pump(6000);
        let st = state().unwrap();
        assert_eq!((st.0, st.1), (Mode::Pinned, false));
        assert_eq!(take_pinned(), vec!["yuno".to_string()]);
        show_auto(build_data("arale", &p, &t, now), anchor);
        assert_eq!(current().as_deref(), Some("yuno"));

        set_topmost(true);
        assert!(ex(h));
        set_topmost(false);
        assert!(!ex(h));
        close();
        pump(400);
    }

    /// 그리기 시간 재기. 실행: cargo test --release perf_bubble -- --ignored --nocapture
    #[test]
    #[ignore]
    fn perf_bubble() {
        let p = parse_preview(include_str!("../fixtures/preview.sample.json")).unwrap();
        let t = parse_tweets(include_str!("../fixtures/tweets.sample.json")).unwrap();
        let now = parse_utc("2026-09-07T12:30:00Z").unwrap();
        let work = work_area();
        show(build_data("arale", &p, &t, now), (work.left + 400, work.bottom - 140));
        pump(200);
        let n = 30;
        // 스크롤(면 그림 캐시 사용)
        let t0 = Instant::now();
        for i in 0..n {
            with_bubble(|b| {
                b.scroll = (i as f32 * 7.0).min(b.max_scroll());
                b.redraw();
            });
        }
        let scroll = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
        // 밀기(두 면 겹치기)
        let id = BUBBLE.with(|c| c.borrow().as_ref().unwrap().lay.cards().last().map(|c| c.tw.id.clone())).unwrap();
        let t1 = Instant::now();
        for i in 0..n {
            with_bubble(|b| {
                let w = b.lay.card(&id).unwrap().w;
                b.scroll = b.max_scroll();
                b.cards.insert(id.clone(), drag_vis(0, -(i as f32) * 6.0, w));
                b.redraw();
            });
        }
        let swipe = t1.elapsed().as_secs_f64() * 1000.0 / n as f64;
        // 면 다시 그리기(캐시 없이)
        let t2 = Instant::now();
        for _ in 0..n {
            with_bubble(|b| {
                b.faces.clear();
                b.redraw();
            });
        }
        let cold = t2.elapsed().as_secs_f64() * 1000.0 / n as f64;
        println!("scroll {scroll:.1} ms · swipe {swipe:.1} ms · cold {cold:.1} ms");
        close();
        pump(300);
    }
}
