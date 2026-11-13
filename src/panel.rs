//! 멤버 패널 — 방송 보러 가기 · 트윗 목록. 명세: docs/IMPL_SPEC.md §8. [담당: 작업 D]

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Once;

use chrono::{DateTime, Utc};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET,
    DEFAULT_PITCH, DRAW_TEXT_FORMAT, DT_CALCRECT, DT_LEFT, DT_NOPREFIX, DT_SINGLELINE,
    DT_WORDBREAK, DeleteObject, DrawTextW, FF_DONTCARE, FW_NORMAL, FillRect, GetDC,
    GetStockObject, HBRUSH, HDC, HFONT, HGDIOBJ, InvalidateRect, OUT_DEFAULT_PRECIS, ReleaseDC,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT, WHITE_BRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, MEASUREITEMSTRUCT, ODS_SELECTED, ODT_LISTBOX};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent, VK_ESCAPE,
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_PUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect, HMENU,
    IDC_ARROW, IsWindow, LB_ADDSTRING, LB_ITEMFROMPOINT, LB_RESETCONTENT, LBS_NOINTEGRALHEIGHT,
    LBS_NOTIFY, LBS_OWNERDRAWVARIABLE, LoadCursorW, MINMAXINFO, MoveWindow, PostMessageW,
    RegisterClassExW, SPI_GETWORKAREA, SW_HIDE, SW_SHOW, SW_SHOWNORMAL, SendMessageW,
    SetForegroundWindow, SetWindowTextW, ShowWindow, SystemParametersInfoW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_CLOSE, WM_COMMAND, WM_CTLCOLORSTATIC, WM_DESTROY, WM_DRAWITEM,
    WM_GETMINMAXINFO, WM_KEYDOWN, WM_LBUTTONUP, WM_MEASUREITEM, WM_MOUSEMOVE, WM_SETFONT,
    WM_SIZE, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_OVERLAPPED, WS_SYSMENU,
    WS_THICKFRAME, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{PCWSTR, w};

/// WM_MOUSELEAVE (TrackMouseEvent 가 보낸다). windows 크레이트 feature 를 늘리지 않으려고 값을 직접 쓴다.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// SS_CENTER (정적 글자 가운데 맞춤). 같은 이유로 값을 직접 쓴다.
const SS_CENTER: u32 = 0x0000_0001;

use crate::model::{Preview, TweetsFile, short_name_ko};
use crate::util::{APP_NAME, fmt_kst, open_url, parse_utc, wide};

/// 번역 전 글에 쓰는 문구(웹과 같음).
const PENDING_TEXT: &str = "번역 준비 중…";
/// 패널에 올리는 트윗 최대 수(웹 MAX_THREAD 와 같음).
const MAX_TWEETS: usize = 50;

const ID_BUTTON: usize = 1001;
const ID_LIST: usize = 1002;
const ID_EMPTY: usize = 1003;
const ID_FOOTER: usize = 1004;

#[derive(Debug, Clone, PartialEq)]
pub struct PanelTweet {
    /// 번역, 없으면 "번역 준비 중…"
    pub text: String,
    pub url: Option<String>,
    /// KST 표기(`11/13 (금) 16:27`), 모르면 빈 문자열
    pub when: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelData {
    pub member: String,
    pub name_ko: String,
    /// 방송 중이면 (제목, 주소)
    pub live: Option<(String, String)>,
    /// 최신이 앞
    pub tweets: Vec<PanelTweet>,
}

/// 순수 함수 — 패널에 보일 내용을 만든다.
pub fn build_data(member: &str, preview: &Preview, tweets: &TweetsFile, now: DateTime<Utc>) -> PanelData {
    // 이름: 데이터에 없어 key 가 그대로 오면 짧은 이름으로.
    let name_ko = {
        let n = preview.name_ko(member);
        if n == member { short_name_ko(member).to_string() } else { n }
    };

    // 방송 중: 이 멤버가 나오는 첫 live 항목(합동 포함).
    let live = preview
        .items
        .iter()
        .find(|i| i.state == "live" && i.members().iter().any(|m| m == member))
        .map(|item| {
            let url = item
                .url
                .clone()
                .filter(|u| !u.trim().is_empty())
                .or_else(|| preview.channel_url(&item.channel_key))
                .unwrap_or_else(|| "https://www.youtube.com/".to_string());
            (item.display_title(), url)
        });

    // 트윗: 만료 안 된 글, 최신이 앞, 최대 50건. 파일은 최신이 뒤.
    let list: &[_] = tweets.tweets.get(member).map(|v| v.as_slice()).unwrap_or(&[]);
    let out: Vec<PanelTweet> = list
        .iter()
        .rev()
        .filter(|t| {
            t.expires_at
                .as_deref()
                .and_then(parse_utc)
                .is_none_or(|exp| exp > now)
        })
        .take(MAX_TWEETS)
        .map(|t| PanelTweet {
            text: t.translated().unwrap_or(PENDING_TEXT).to_string(),
            url: Some(
                t.url
                    .clone()
                    .filter(|u| !u.trim().is_empty())
                    .unwrap_or_else(|| format!("https://x.com/i/status/{}", t.id)),
            ),
            when: t
                .posted_at
                .as_deref()
                .and_then(fmt_kst)
                .or_else(|| t.received_at.as_deref().and_then(fmt_kst))
                .unwrap_or_default(),
        })
        .collect();

    PanelData { member: member.to_string(), name_ko, live, tweets: out }
}

// ---------------------------------------------------------------------------
// 창 상태 — 메인 스레드 전용. 상태를 고칠 때만 borrow_mut 을 쓰고, Win32 호출 중엔 읽기만 한다.

#[derive(Clone, Copy)]
struct Handles {
    hwnd: HWND,
    list: HWND,
    btn: HWND,
    empty: HWND,
    footer: HWND,
    font: HFONT,
    font_small: HFONT,
}

struct Panel {
    h: Handles,
    scale: f32,
    data: PanelData,
    hover: Option<usize>,
}

thread_local! {
    static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) };
}

fn with_panel<R>(f: impl FnOnce(&Panel) -> R) -> Option<R> {
    PANEL.with(|c| c.borrow().as_ref().map(f))
}

fn px(n: i32, s: f32) -> i32 {
    (n as f32 * s).round() as i32
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

fn hinstance() -> HINSTANCE {
    unsafe { GetModuleHandleW(None) }
        .map(|m| HINSTANCE(m.0))
        .unwrap_or_default()
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            lpszClassName: w!("MewFriendsPanel"),
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
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut r as *mut RECT as *mut c_void),
            Default::default(),
        );
    }
    r
}

fn create_font(height: i32) -> HFONT {
    unsafe {
        CreateFontW(
            height,
            0,
            0,
            0,
            FW_NORMAL.0 as i32,
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

/// 패널을 띄운다(이미 떠 있으면 내용을 바꾸고 앞으로). 메인 스레드에서 부른다.
pub fn show(data: PanelData) {
    ensure_class();

    if let Some(hwnd) = with_panel(|p| p.h.hwnd) {
        if unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            // 데이터만 바꾸고(빌림 해제 후) 다시 맞춘다.
            PANEL.with(|c| {
                if let Some(p) = c.borrow_mut().as_mut() {
                    p.data = data;
                    p.hover = None;
                }
            });
            refresh();
            unsafe {
                // 최소화돼 있던 경우도 되살리려고 SW_SHOW 가 아니라 SW_SHOWNORMAL 을 쓴다.
                let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
                let _ = SetForegroundWindow(hwnd);
            }
            return;
        }
    }

    let dpi = unsafe { GetDpiForSystem() };
    let scale = (dpi as f32 / 96.0).max(0.5);
    let work = work_area();
    let w = px(360, scale);
    let h = px(480, scale);
    let margin = px(16, scale);
    let x = (work.right - w - margin).max(work.left);
    let y = (work.bottom - h - margin).max(work.top);
    let title = wide(&format!("{} — {}", data.name_ko, APP_NAME));
    let hi = hinstance();

    let hwnd = unsafe {
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("MewFriendsPanel"),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME,
            x,
            y,
            w,
            h,
            None,
            None,
            Some(hi),
            None,
        ) {
            Ok(hwnd) => hwnd,
            Err(e) => {
                eprintln!("[panel] 창 생성 실패: {e}");
                return;
            }
        }
    };

    let font = create_font(-px(15, scale));
    let font_small = create_font(-px(12, scale));
    let child = |class: PCWSTR, text: &str, style: WINDOW_STYLE, id: usize| unsafe {
        let t = wide(text);
        let c = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(t.as_ptr()),
            WS_CHILD | style,
            0,
            0,
            1,
            1,
            Some(hwnd),
            Some(HMENU(id as *mut c_void)),
            Some(hi),
            None,
        )
        .unwrap_or_default();
        SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        c
    };
    // BS_LEFT(0x100): 긴 제목은 가운데 정렬이면 양쪽이 잘리므로 왼쪽부터 보여 준다.
    let btn = child(w!("BUTTON"), "", WS_VISIBLE | WINDOW_STYLE(BS_PUSHBUTTON as u32 | 0x0100), ID_BUTTON);
    let list = child(
        w!("LISTBOX"),
        "",
        WS_VISIBLE
            | WS_VSCROLL
            | WS_BORDER
            | WINDOW_STYLE(LBS_OWNERDRAWVARIABLE as u32)
            | WINDOW_STYLE(LBS_NOINTEGRALHEIGHT as u32)
            | WINDOW_STYLE(LBS_NOTIFY as u32),
        ID_LIST,
    );
    let empty = child(w!("STATIC"), "표시할 트윗이 없어요", WINDOW_STYLE(SS_CENTER), ID_EMPTY);
    let footer = child(w!("STATIC"), "비공식 팬 제작 · 공식과 무관", WS_VISIBLE, ID_FOOTER);

    unsafe {
        let _ = SetWindowSubclass(list, Some(child_proc), 1, 0);
        let _ = SetWindowSubclass(btn, Some(child_proc), 2, 0);
    }

    let h = Handles { hwnd, list, btn, empty, footer, font, font_small };
    PANEL.with(|c| *c.borrow_mut() = Some(Panel { h, scale, data, hover: None }));
    refresh();
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
    }
}

/// 현재 데이터 · 크기에 맞춰 컨트롤 배치 · 글 목록 채우기 · 제목 갱신.
/// 패널이 없으면(생성 중 WM_SIZE 등) 아무것도 하지 않는다.
fn refresh() {
    with_panel(|p| {
        let h = p.h;
        let s = p.scale;
        let mut rc = RECT::default();
        unsafe {
            let _ = GetClientRect(h.hwnd, &mut rc);
        }
        let pad = px(10, s);
        let (cw, ch) = (rc.right, rc.bottom);
        let w = (cw - 2 * pad).max(0);
        let foot_h = px(20, s);
        let live = p.data.live.clone();
        let btn_h = if live.is_some() { px(30, s) } else { 0 };
        let gap = if live.is_some() { px(8, s) } else { 0 };
        let list_y = pad + btn_h + gap;
        let foot_y = ch - pad - foot_h;
        let list_h = (foot_y - px(6, s) - list_y).max(0);
        let has_tweets = !p.data.tweets.is_empty();

        unsafe {
            let title = wide(&format!("{} — {}", p.data.name_ko, APP_NAME));
            let _ = SetWindowTextW(h.hwnd, PCWSTR(title.as_ptr()));

            let _ = MoveWindow(h.footer, pad, foot_y, w, foot_h, true);
            let _ = MoveWindow(h.btn, pad, pad, w, btn_h, true);
            let _ = ShowWindow(h.btn, if live.is_some() { SW_SHOW } else { SW_HIDE });
            if let Some((title, _)) = &live {
                let t = wide(&format!("▶ 방송 보러 가기 — {title}"));
                let _ = SetWindowTextW(h.btn, PCWSTR(t.as_ptr()));
            }
            let _ = MoveWindow(h.list, pad, list_y, w, list_h, true);
            let _ = MoveWindow(h.empty, pad, list_y, w, list_h, true);
            let _ = ShowWindow(h.list, if has_tweets { SW_SHOW } else { SW_HIDE });
            let _ = ShowWindow(h.empty, if has_tweets { SW_HIDE } else { SW_SHOW });

            // 글 목록을 다시 채운다. 폭이 바뀌면 줄바꿈 높이가 달라지므로 항목마다 WM_MEASUREITEM 을 다시 받는다.
            SendMessageW(h.list, LB_RESETCONTENT, None, None);
            for i in 0..p.data.tweets.len() {
                SendMessageW(h.list, LB_ADDSTRING, None, Some(LPARAM(i as isize)));
            }
            let _ = InvalidateRect(Some(h.list), None, false);
        }
    });
}

/// 글 하나의 높이 = 위 여백 + 시각 줄 + 간격 + 본문 + 아래 여백.
fn item_height(p: &Panel, t: &PanelTweet, list_w: i32) -> i32 {
    let s = p.scale;
    let inner = (list_w - 2 * px(12, s)).max(1);
    let pad_y = px(8, s);
    let time_h = if t.when.is_empty() {
        0
    } else {
        measure_text(p.h.list, p.h.font_small, &t.when, inner, DRAW_TEXT_FORMAT(0)) + px(4, s)
    };
    let body_h = measure_text(p.h.list, p.h.font, &t.text, inner, DT_WORDBREAK);
    pad_y * 2 + time_h + body_h
}

fn measure_text(hwnd: HWND, font: HFONT, text: &str, width: i32, extra: DRAW_TEXT_FORMAT) -> i32 {
    unsafe {
        let hdc = GetDC(Some(hwnd));
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let mut buf: Vec<u16> = text.encode_utf16().collect();
        if buf.is_empty() {
            buf.push(b' ' as u16);
        }
        let mut rc = RECT { left: 0, top: 0, right: width.max(1), bottom: 0 };
        DrawTextW(hdc, &mut buf, &mut rc, DT_CALCRECT | DT_NOPREFIX | extra);
        SelectObject(hdc, old);
        ReleaseDC(Some(hwnd), hdc);
        (rc.bottom - rc.top).max(1)
    }
}

fn on_measure(lp: LPARAM) {
    let mis = lp.0 as *mut MEASUREITEMSTRUCT;
    if mis.is_null() {
        return;
    }
    unsafe {
        if (*mis).CtlType.0 != ODT_LISTBOX.0 {
            return;
        }
        let idx = (*mis).itemID as usize;
        let h = with_panel(|p| {
            let mut rc = RECT::default();
            let _ = GetClientRect(p.h.list, &mut rc);
            match p.data.tweets.get(idx) {
                Some(t) => item_height(p, t, rc.right),
                None => px(16, p.scale),
            }
        })
        .unwrap_or(40);
        (*mis).itemHeight = h as u32;
    }
}

fn on_draw(lp: LPARAM) {
    let dis = lp.0 as *mut DRAWITEMSTRUCT;
    if dis.is_null() {
        return;
    }
    unsafe {
        if (*dis).CtlType.0 != ODT_LISTBOX.0 {
            return;
        }
        let d = &*dis;
        let idx = d.itemID as usize;
        with_panel(|p| {
            let Some(t) = p.data.tweets.get(idx) else { return };
            let s = p.scale;
            let hot = p.hover == Some(idx) || (d.itemState.0 & ODS_SELECTED.0) != 0;
            let hdc: HDC = d.hDC;
            let rc = d.rcItem;

            let bg = CreateSolidBrush(if hot { rgb(253, 231, 240) } else { rgb(255, 255, 255) });
            FillRect(hdc, &rc, bg);
            let _ = DeleteObject(HGDIOBJ(bg.0));

            let pad_x = px(12, s);
            let pad_y = px(8, s);
            let inner_w = (rc.right - rc.left - 2 * pad_x).max(1);
            let mut y = rc.top + pad_y;
            SetBkMode(hdc, TRANSPARENT);

            if !t.when.is_empty() {
                let th = measure_text(p.h.list, p.h.font_small, &t.when, inner_w, DRAW_TEXT_FORMAT(0));
                SelectObject(hdc, HGDIOBJ(p.h.font_small.0));
                SetTextColor(hdc, rgb(0x88, 0x88, 0x88));
                let mut tr = RECT { left: rc.left + pad_x, top: y, right: rc.right - pad_x, bottom: y + th };
                let mut wt: Vec<u16> = t.when.encode_utf16().collect();
                DrawTextW(hdc, &mut wt, &mut tr, DT_LEFT | DT_SINGLELINE | DT_NOPREFIX);
                y += th + px(4, s);
            }

            SelectObject(hdc, HGDIOBJ(p.h.font.0));
            let pending = t.text == PENDING_TEXT;
            SetTextColor(hdc, if pending { rgb(0x88, 0x88, 0x88) } else { rgb(0x22, 0x22, 0x22) });
            let bh = measure_text(p.h.list, p.h.font, &t.text, inner_w, DT_WORDBREAK);
            let mut br = RECT { left: rc.left + pad_x, top: y, right: rc.right - pad_x, bottom: y + bh };
            let mut wb: Vec<u16> = t.text.encode_utf16().collect();
            if wb.is_empty() {
                wb.push(b' ' as u16);
            }
            DrawTextW(hdc, &mut wb, &mut br, DT_WORDBREAK | DT_NOPREFIX);
        });
    }
}

/// 목록에서 좌표(lparam)의 글 번호. 항목 바깥이면 None.
fn hit_test(list: HWND, lparam: LPARAM) -> Option<usize> {
    let r = unsafe { SendMessageW(list, LB_ITEMFROMPOINT, Some(WPARAM(0)), Some(lparam)) };
    let index = (r.0 & 0xFFFF) as usize;
    let outside = ((r.0 >> 16) & 0xFFFF) != 0;
    if outside { None } else { Some(index) }
}

fn post_close() {
    if let Some(hwnd) = with_panel(|p| p.h.hwnd) {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

fn set_hover(list: HWND, new: Option<usize>) {
    let changed = PANEL.with(|c| {
        let mut b = c.borrow_mut();
        match b.as_mut() {
            Some(p) if p.hover != new => {
                p.hover = new;
                true
            }
            _ => false,
        }
    });
    if changed {
        unsafe {
            let _ = InvalidateRect(Some(list), None, false);
        }
    }
}

/// 목록 · 버튼 서브클래스 — 마우스 강조 · 마우스 클릭으로 열기 · Esc 닫기.
unsafe extern "system" fn child_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_KEYDOWN && wp.0 == VK_ESCAPE.0 as usize {
            post_close();
            return LRESULT(0);
        }
        let is_list = with_panel(|p| p.h.list == hwnd).unwrap_or(false);
        if is_list {
            match msg {
                WM_MOUSEMOVE => {
                    let new = hit_test(hwnd, lp);
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut tme);
                    set_hover(hwnd, new);
                }
                WM_MOUSELEAVE => set_hover(hwnd, None),
                WM_LBUTTONUP => {
                    // LBN_SELCHANGE 는 화살표 키 이동에도 난다. 그러면 키 누를 때마다 브라우저가 뜨므로
                    // 마우스로 누른 항목에서만 연다.
                    if let Some(i) = hit_test(hwnd, lp) {
                        let url = with_panel(|p| p.data.tweets.get(i).and_then(|t| t.url.clone()))
                            .flatten();
                        if let Some(u) = url {
                            open_url(&u);
                        }
                    }
                }
                _ => {}
            }
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_SIZE => {
                refresh();
                LRESULT(0)
            }
            WM_GETMINMAXINFO => {
                let s = with_panel(|p| p.scale).unwrap_or(1.0);
                let mm = lp.0 as *mut MINMAXINFO;
                if !mm.is_null() {
                    (*mm).ptMinTrackSize = POINT { x: px(240, s), y: px(300, s) };
                }
                LRESULT(0)
            }
            WM_MEASUREITEM => {
                on_measure(lp);
                LRESULT(1)
            }
            WM_DRAWITEM => {
                on_draw(lp);
                LRESULT(1)
            }
            WM_CTLCOLORSTATIC => {
                // 설명 글자는 회색, 배경은 흰색 그대로.
                let hdc = HDC(wp.0 as *mut c_void);
                SetTextColor(hdc, rgb(0x88, 0x88, 0x88));
                SetBkMode(hdc, TRANSPARENT);
                LRESULT(GetStockObject(WHITE_BRUSH).0 as isize)
            }
            WM_COMMAND => {
                // 버튼 클릭(BN_CLICKED = 0) → 방송 주소
                let id = wp.0 & 0xFFFF;
                let code = (wp.0 >> 16) & 0xFFFF;
                if id == ID_BUTTON && code == 0 {
                    let url = with_panel(|p| p.data.live.as_ref().map(|(_, u)| u.clone())).flatten();
                    if let Some(u) = url {
                        open_url(&u);
                    }
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if wp.0 == VK_ESCAPE.0 as usize {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                // 닫히면 상태를 비운다(다음 show 때 새로 만든다). 메인 메시지 루프는 그대로 돈다.
                let taken = PANEL.with(|c| c.borrow_mut().take());
                if let Some(p) = taken {
                    let _ = DeleteObject(HGDIOBJ(p.h.font.0));
                    let _ = DeleteObject(HGDIOBJ(p.h.font_small.0));
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
    use crate::model::{parse_preview, parse_tweets};
    use std::time::{Duration, Instant};
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, GetDIBits,
    };
    use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, GetWindowRect, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
    };

    fn at(s: &str) -> DateTime<Utc> {
        parse_utc(s).unwrap()
    }

    const NOW: &str = "2026-11-13T12:00:00Z";

    #[test]
    fn expired_hidden_newest_first() {
        let t = parse_tweets(
            r#"{"tweets":{"yuno":[
              {"id":"1","text_ko":"오래된","expires_at":"2026-11-14T00:00:00Z","received_at":"2026-11-10T00:00:00Z"},
              {"id":"2","text_ko":"만료","expires_at":"2026-11-12T00:00:00Z","posted_at":"2026-11-11T00:00:00Z"},
              {"id":"3","text_ko":"새 글","expires_at":"2026-11-14T00:00:00Z","posted_at":"2026-11-13T11:00:00Z"}
            ]}}"#,
        )
        .unwrap();
        let d = build_data("yuno", &Preview::default(), &t, at(NOW));
        let texts: Vec<&str> = d.tweets.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, vec!["새 글", "오래된"]);
        // 2026-11-13T11:00Z = KST 20:00
        assert_eq!(d.tweets[0].when, "11/13 (금) 20:00");
    }

    #[test]
    fn untranslated_and_time_priority() {
        let t = parse_tweets(
            r#"{"tweets":{"yuno":[
              {"id":"9","text":"あ","received_at":"2026-11-13T10:00:00Z"},
              {"id":"8","text_ko":"  ","posted_at":"2026-11-13T09:00:00Z","received_at":"2026-11-13T10:00:00Z"},
              {"id":"7","text_ko":"x"}
            ]}}"#,
        )
        .unwrap();
        let d = build_data("yuno", &Preview::default(), &t, at(NOW));
        // 뒤에서부터(최신이 앞): 7, 8, 9
        assert_eq!(d.tweets[0].text, "x");
        assert_eq!(d.tweets[0].when, "");
        assert_eq!(d.tweets[1].text, PENDING_TEXT);
        // posted_at 우선 (09:00Z = KST 18:00)
        assert_eq!(d.tweets[1].when, "11/13 (금) 18:00");
        // posted_at 없으면 received_at (10:00Z = KST 19:00)
        assert_eq!(d.tweets[2].when, "11/13 (금) 19:00");
        assert_eq!(d.tweets[2].url.as_deref(), Some("https://x.com/i/status/9"));
    }

    #[test]
    fn limit_50() {
        let items: Vec<String> = (0..60)
            .map(|i| format!(r#"{{"id":"{i}","text_ko":"t{i}"}}"#))
            .collect();
        let t = parse_tweets(&format!(r#"{{"tweets":{{"yuno":[{}]}}}}"#, items.join(","))).unwrap();
        let d = build_data("yuno", &Preview::default(), &t, at(NOW));
        assert_eq!(d.tweets.len(), 50);
        assert_eq!(d.tweets[0].text, "t59");
    }

    #[test]
    fn live_and_collab_and_fallback_url() {
        let p = parse_preview(
            r#"{"channels":{"yuno":{"name_ko":"유노","handle":"yuno_h"}},"items":[
              {"id":"a","state":"upcoming","channel_key":"yuno","title_ko":"예고"},
              {"id":"b","state":"live","channel_key":"nonoka","collab_with":["yuno"],"title_ko":"합동 방송"}
            ]}"#,
        )
        .unwrap();
        let t = TweetsFile::default();
        let d = build_data("yuno", &p, &t, at(NOW));
        // 합동 live 는 yuno 에도 뜬다. url 없음 · nonoka 채널 정보 없음 → 기본 주소.
        assert_eq!(
            d.live,
            Some(("합동 방송".to_string(), "https://www.youtube.com/".to_string()))
        );
        assert_eq!(d.name_ko, "유노");
        // upcoming 은 live 가 아님
        let p2 = parse_preview(
            r#"{"channels":{"yuno":{"handle":"yuno_h"}},"items":[{"id":"a","state":"upcoming","channel_key":"yuno"}]}"#,
        )
        .unwrap();
        assert_eq!(build_data("yuno", &p2, &t, at(NOW)).live, None);
        // url 없는 live + 채널 handle 있음 → 채널 주소
        let p3 = parse_preview(
            r#"{"channels":{"yuno":{"handle":"yuno_h"}},"items":[{"id":"a","state":"live","channel_key":"yuno","url":null,"title":"T"}]}"#,
        )
        .unwrap();
        assert_eq!(
            build_data("yuno", &p3, &t, at(NOW)).live,
            Some(("T".to_string(), "https://www.youtube.com/@yuno_h".to_string()))
        );
    }

    #[test]
    fn empty_data_uses_short_name() {
        let d = build_data("miyako", &Preview::default(), &TweetsFile::default(), at(NOW));
        assert_eq!(d.name_ko, "미야코");
        assert!(d.tweets.is_empty());
        assert!(d.live.is_none());
    }

    /// 창을 띄우고 3초 메시지 루프를 돌린 뒤 `target/panel_shot.png` 로 저장한다.
    /// 실행: cargo test shot_panel -- --ignored --nocapture
    #[test]
    #[ignore]
    fn shot_panel() {
        let p = parse_preview(include_str!("../fixtures/preview.sample.json")).unwrap();
        // fixture 의 "ritsu" 는 옛 단건 객체 — model 이 배열로 감싸 읽는다(SPEC 계약 I 하위호환).
        let t = parse_tweets(include_str!("../fixtures/tweets.sample.json")).unwrap();
        let member = p
            .items
            .iter()
            .find(|i| i.state == "live")
            .map(|i| i.channel_key.clone())
            .unwrap_or_else(|| "arale".to_string());
        let now = Utc::now();
        let mut data = build_data(&member, &p, &t, now);
        // 줄바꿈 확인용: 같은 시험 fixture 의 다른 멤버 글을 뒤에 덧붙인다(메모리에서만).
        data.tweets.extend(build_data("arale", &p, &t, now).tweets);
        println!("member={member} live={:?} tweets={}", data.live, data.tweets.len());
        show(data);

        let start = Instant::now();
        let mut msg = MSG::default();
        while start.elapsed() < Duration::from_secs(3) {
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            std::thread::sleep(Duration::from_millis(16));
        }

        let hwnd = with_panel(|p| p.h.hwnd).expect("패널이 떠 있어야 한다");
        let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/panel_shot.png");
        capture(hwnd, &out);
        println!("saved {}", out.display());
    }

    fn capture(hwnd: HWND, path: &std::path::Path) {
        unsafe {
            let mut rc = RECT::default();
            GetWindowRect(hwnd, &mut rc).unwrap();
            let (w, h) = (rc.right - rc.left, rc.bottom - rc.top);
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            // PW_RENDERFULLCONTENT = 2
            let _ = PrintWindow(hwnd, mem, PRINT_WINDOW_FLAGS(2));
            let mut bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut buf = vec![0u8; (w * h * 4) as usize];
            GetDIBits(mem, bmp, 0, h as u32, Some(buf.as_mut_ptr() as *mut c_void), &mut bi, DIB_RGB_COLORS);
            for px in buf.chunks_exact_mut(4) {
                px.swap(0, 2);
                px[3] = 255;
            }
            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            if let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, buf) {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = img.save(path) {
                    eprintln!("[panel] 캡처 저장 실패: {e}");
                }
            }
        }
    }
}
