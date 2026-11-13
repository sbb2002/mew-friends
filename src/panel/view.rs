//! 말풍선 트윗 시트 그리기 — mewtype-scheduler 트윗 시트(css/tweets.css `.tws`)를 옮긴다.
//!
//! 바탕(그림자 · 그라데이션 · 꼬리)과 머리 줄은 따로 그려 두고, 글 카드는 면(번역 · 원문)마다 한 장씩 그려 두었다가(card.rs)
//! 스크롤 · 밀기 때는 붙이기만 한다. 글자는 GDI 로 흑백 덮임 정도(커버리지)를 받아 색을 입혀 섞는다 — 그래서 반투명 바탕 ·
//! 투명도 움직임 위에서도 그릴 수 있다. 치수는 모두 CSS px × DPI 배율.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use tiny_skia::{
    Color, FillRule, FilterQuality, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint,
    Path, PathBuilder, Pattern, Pixmap, PixmapPaint, Point, Rect, Shader, SpreadMode, Stroke,
    Transform,
};
use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    ANTIALIASED_QUALITY, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, DEFAULT_CHARSET, DEFAULT_PITCH, DIB_RGB_COLORS,
    DRAW_TEXT_FORMAT, DT_CALCRECT, DT_NOPREFIX, DT_SINGLELINE, DeleteDC, DeleteObject, DrawTextW,
    FF_DONTCARE, FW_BOLD, FW_NORMAL, GetDC, HBITMAP, HDC, HFONT, HGDIOBJ, IntersectClipRect,
    OUT_DEFAULT_PRECIS, ReleaseDC, SelectClipRgn, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::core::w;

// ---------------------------------------------------------------------------
// 치수 (CSS px)

/// 말풍선 폭(원본 PC 팝업 540 · 폰 시트 최대 460).
pub const BUBBLE_W: f32 = 480.0;
/// 그림자 여백(창 가장자리 ~ 말풍선).
pub const SHADOW: f32 = 18.0;
/// 꼬리 높이 · 반폭.
pub const TAIL_H: f32 = 16.0;
pub const TAIL_HALF: f32 = 13.0;
pub const RADIUS: f32 = 20.0;

// ---------------------------------------------------------------------------
// 색

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub f32, pub f32, pub f32);

impl Rgb {
    /// color-mix(self t, o)
    pub fn mix(self, o: Rgb, t: f32) -> Rgb {
        Rgb(self.0 * t + o.0 * (1.0 - t), self.1 * t + o.1 * (1.0 - t), self.2 * t + o.2 * (1.0 - t))
    }
    pub fn color(self, a: f32) -> Color {
        Color::from_rgba(
            (self.0 / 255.0).clamp(0.0, 1.0),
            (self.1 / 255.0).clamp(0.0, 1.0),
            (self.2 / 255.0).clamp(0.0, 1.0),
            a.clamp(0.0, 1.0),
        )
        .unwrap_or(Color::BLACK)
    }
    pub const fn hex(v: u32) -> Rgb {
        Rgb(((v >> 16) & 0xff) as f32, ((v >> 8) & 0xff) as f32, (v & 0xff) as f32)
    }
}

pub const WHITE: Rgb = Rgb(255.0, 255.0, 255.0);
pub const BLACK: Rgb = Rgb(0.0, 0.0, 0.0);
pub const INK_DARK: Rgb = Rgb(23.0, 23.0, 27.0);
const FALLBACK_BG: Rgb = Rgb(58.0, 58.0, 62.0);

/// X 카드(어둡게) 색.
pub mod xc {
    use super::Rgb;
    pub const BG: Rgb = Rgb::hex(0x000000);
    pub const TEXT: Rgb = Rgb::hex(0xe7e9ea);
    pub const SUB: Rgb = Rgb::hex(0x71767b);
    pub const LINE: Rgb = Rgb::hex(0x2f3336);
    pub const PILL: Rgb = Rgb::hex(0x536471);
    pub const BLUE: Rgb = Rgb::hex(0x1d9bf0);
    pub const CELL: Rgb = Rgb::hex(0x16181c);
    pub const HOVER: Rgb = Rgb::hex(0x181919);
}

fn lum(c: Rgb) -> f32 {
    let f = |v: f32| {
        let v = v / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * f(c.0) + 0.7152 * f(c.1) + 0.0722 * f(c.2)
}

fn contrast(a: f32, b: f32) -> f32 {
    let (hi, lo) = if a > b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// 시트 색 묶음(tweetlogic.js paletteFor + tweets.css `.tws` 토큰).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// 바탕 = 멤버 색(--m)
    pub m: Rgb,
    /// 글자 = 검정 · 흰색 중 대비 큰 쪽(--mi)
    pub mi: Rgb,
    pub dark_ink: bool,
}

impl Palette {
    pub fn new(m: Option<Rgb>) -> Palette {
        let m = m.unwrap_or(FALLBACK_BG);
        let l = lum(m);
        let dark_ink = contrast(l, lum(INK_DARK)) >= contrast(l, 1.0);
        Palette { m, mi: if dark_ink { INK_DARK } else { WHITE }, dark_ink }
    }
    /// --sub = color-mix(mi 84%, m)
    pub fn sub(&self) -> Rgb {
        self.mi.mix(self.m, 0.84)
    }
    /// --paper: 검정 글자면 흰 38%, 흰 글자면 검정 20%
    pub fn paper(&self) -> Color {
        if self.dark_ink { WHITE.color(0.38) } else { BLACK.color(0.2) }
    }
    pub fn mi_a(&self, a: f32) -> Color {
        self.mi.color(a)
    }
    pub fn key(&self) -> String {
        format!("{:?}", self.m)
    }
}

/// 아바타 평균색(예고판 sampleLaneColor 와 같은 뜻 — 투명한 곳은 뺀다).
pub fn avg_color(img: &Pixmap) -> Option<Rgb> {
    let (mut r, mut g, mut b, mut n) = (0f64, 0f64, 0f64, 0f64);
    for p in img.pixels() {
        let c = p.demultiply();
        let a = c.alpha() as f64 / 255.0;
        r += c.red() as f64 * a;
        g += c.green() as f64 * a;
        b += c.blue() as f64 * a;
        n += a;
    }
    if n < 1.0 {
        return None;
    }
    Some(Rgb((r / n).floor() as f32, (g / n).floor() as f32, (b / n).floor() as f32))
}

// ---------------------------------------------------------------------------
// 아바타

pub struct Avatar {
    pub key: String,
    pub img: Pixmap,
    pub gray: Pixmap,
    pub color: Option<Rgb>,
}

fn icon_bytes(key: &str) -> Option<&'static [u8]> {
    match key {
        "arale" => Some(include_bytes!("../../assets/icons/arale.png")),
        "yuno" => Some(include_bytes!("../../assets/icons/yuno.png")),
        "nonoka" => Some(include_bytes!("../../assets/icons/nonoka.png")),
        "ritsu" => Some(include_bytes!("../../assets/icons/ritsu.png")),
        "miyako" => Some(include_bytes!("../../assets/icons/miyako.png")),
        _ => None,
    }
}

pub fn load_avatar(key: &str) -> Option<Avatar> {
    let bytes = icon_bytes(key)?;
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (w, h) = img.dimensions();
    let mut pm = Pixmap::new(w, h)?;
    let mut gray = Pixmap::new(w, h)?;
    for ((s, d), g) in img
        .as_raw()
        .chunks_exact(4)
        .zip(pm.data_mut().chunks_exact_mut(4))
        .zip(gray.data_mut().chunks_exact_mut(4))
    {
        let a = s[3] as u32;
        let pre = |v: u8| ((v as u32 * a + 127) / 255) as u8;
        d.copy_from_slice(&[pre(s[0]), pre(s[1]), pre(s[2]), s[3]]);
        // grayscale(0.6)
        let y = 0.2126 * s[0] as f32 + 0.7152 * s[1] as f32 + 0.0722 * s[2] as f32;
        let gm = |v: u8| pre((v as f32 * 0.4 + y * 0.6).round().clamp(0.0, 255.0) as u8);
        g.copy_from_slice(&[gm(s[0]), gm(s[1]), gm(s[2]), s[3]]);
    }
    let color = avg_color(&pm);
    Some(Avatar { key: key.to_string(), img: pm, gray, color })
}

// ---------------------------------------------------------------------------
// 글꼴 · 재기

/// 크기 · 굵기별 글꼴(처음 쓸 때 만든다). 흑백 안티에일리어싱(커버리지로 쓴다).
pub struct Fonts {
    s: f32,
    map: RefCell<HashMap<(u32, bool), HFONT>>,
}

impl Fonts {
    pub fn new(s: f32) -> Fonts {
        Fonts { s, map: RefCell::new(HashMap::new()) }
    }
    /// size = CSS px, bold = 굵게(700~800)
    pub fn get(&self, size: f32, bold: bool) -> HFONT {
        let key = ((size * 10.0).round() as u32, bold);
        *self.map.borrow_mut().entry(key).or_insert_with(|| unsafe {
            CreateFontW(
                -((size * self.s).round() as i32),
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
                ANTIALIASED_QUALITY,
                (DEFAULT_PITCH.0 as u32) | (FF_DONTCARE.0 as u32),
                w!("Malgun Gothic"),
            )
        })
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for f in self.map.borrow().values() {
            unsafe {
                let _ = DeleteObject(HGDIOBJ(f.0));
            }
        }
    }
}

/// 글자 재기용 메모리 DC.
pub struct Measurer {
    dc: HDC,
}

impl Measurer {
    pub fn new() -> Measurer {
        Measurer { dc: unsafe { CreateCompatibleDC(None) } }
    }
    pub fn width(&self, f: HFONT, text: &str) -> f32 {
        if text.is_empty() {
            return 0.0;
        }
        unsafe {
            let old = SelectObject(self.dc, HGDIOBJ(f.0));
            let mut buf: Vec<u16> = text.encode_utf16().collect();
            let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            DrawTextW(self.dc, &mut buf, &mut rc, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
            SelectObject(self.dc, old);
            (rc.right - rc.left) as f32
        }
    }

    /// 줄바꿈 → 줄마다 text 안 바이트 범위. 띄어쓰기에서 나누고(keep-all), 한 덩어리가 폭을 넘으면 글자 단위
    /// (일본어처럼 띄어쓰기 없는 글은 글자 단위로 채워진다). 빈 줄도 한 줄.
    pub fn wrap_idx(&self, f: HFONT, text: &str, max_w: f32) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let trim = |s: usize, mut e: usize| {
            while e > s && matches!(text.as_bytes()[e - 1], b' ' | b'\t' | b'\r') {
                e -= 1;
            }
            (s, e)
        };
        let mut base = 0usize;
        for para in text.split('\n') {
            let pbase = base;
            base += para.len() + 1;
            let mut ls: Option<usize> = None;
            let mut le = pbase;
            for (off, tok) in tokens(para) {
                let ts = pbase + off;
                let te = ts + tok.len();
                let start = ls.unwrap_or(ts);
                if self.width(f, text[start..te].trim_end()) <= max_w {
                    ls = Some(start);
                    le = te;
                    continue;
                }
                if let Some(s0) = ls.take() {
                    if !text[s0..le].trim().is_empty() {
                        out.push(trim(s0, le));
                    }
                }
                let tstart = ts + (tok.len() - tok.trim_start().len());
                if self.width(f, text[tstart..te].trim_end()) <= max_w {
                    ls = Some(tstart);
                    le = te;
                } else {
                    let mut cs = tstart;
                    for (ci, ch) in text[tstart..te].char_indices() {
                        let at = tstart + ci;
                        let cend = at + ch.len_utf8();
                        if at > cs && self.width(f, &text[cs..cend]) > max_w {
                            out.push((cs, at));
                            cs = at;
                        }
                    }
                    ls = Some(cs);
                    le = te;
                }
            }
            match ls {
                Some(s0) => out.push(trim(s0, le)),
                None => out.push((pbase, pbase)),
            }
        }
        out
    }

    pub fn wrap(&self, f: HFONT, text: &str, max_w: f32) -> Vec<String> {
        self.wrap_idx(f, text, max_w).into_iter().map(|(s, e)| text[s..e].to_string()).collect()
    }
}

impl Drop for Measurer {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.dc);
        }
    }
}

/// "가 나  다" → [(0,"가 "), (4,"나  "), (9,"다")]
fn tokens(s: &str) -> Vec<(usize, &str)> {
    let mut v = Vec::new();
    let mut start = 0;
    let mut in_space = false;
    for (i, ch) in s.char_indices() {
        if ch == ' ' || ch == '\t' {
            in_space = true;
        } else if in_space {
            v.push((start, &s[start..i]));
            start = i;
            in_space = false;
        }
    }
    if start < s.len() {
        v.push((start, &s[start..]));
    }
    v
}

/// 링크처럼 보이는 낱말(주소 · #태그 · @멘션)의 바이트 범위 — 원문 면에서 파랗게.
pub fn link_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    let b = text.as_bytes();
    while i < b.len() {
        let rest = &text[i..];
        let starts = rest.starts_with("http://")
            || rest.starts_with("https://")
            || rest.starts_with('#')
            || rest.starts_with('＃')
            || rest.starts_with('@');
        let at_word = i == 0 || text[..i].chars().last().is_some_and(|c| c.is_whitespace() || "（(「『、。".contains(c));
        if starts && at_word {
            let end = rest
                .find(|c: char| c.is_whitespace() || "）)」』、。".contains(c))
                .map(|e| i + e)
                .unwrap_or(text.len());
            if end > i + 1 {
                out.push((i, end));
                i = end;
                continue;
            }
        }
        i += rest.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    out
}

// ---------------------------------------------------------------------------
// 도형

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct R {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl R {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> R {
        R { x, y, w, h }
    }
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
    pub fn off(&self, dx: f32, dy: f32) -> R {
        R::new(self.x + dx, self.y + dy, self.w, self.h)
    }
    pub fn inset(&self, d: f32) -> R {
        R::new(self.x + d, self.y + d, (self.w - 2.0 * d).max(0.0), (self.h - 2.0 * d).max(0.0))
    }
    pub fn rect(&self) -> Option<Rect> {
        Rect::from_xywh(self.x, self.y, self.w.max(0.01), self.h.max(0.01))
    }
    pub fn rc(&self) -> RECT {
        RECT {
            left: self.x.round() as i32,
            top: self.y.round() as i32,
            right: (self.x + self.w).round() as i32,
            bottom: (self.y + self.h).round() as i32,
        }
    }
    pub fn intersect(&self, o: &R) -> Option<R> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        (x1 > x0 && y1 > y0).then(|| R::new(x0, y0, x1 - x0, y1 - y0))
    }
}

/// 모서리마다 반지름(왼쪽 위 · 오른쪽 위 · 오른쪽 아래 · 왼쪽 아래).
pub fn rrect4(r: R, rad: [f32; 4]) -> Option<Path> {
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    let lim = (w / 2.0).min(h / 2.0).max(0.0);
    let [tl, tr, br, bl] = rad.map(|v| v.clamp(0.0, lim));
    let k = 0.552_284_8;
    let mut pb = PathBuilder::new();
    pb.move_to(x + tl, y);
    pb.line_to(x + w - tr, y);
    if tr > 0.0 {
        pb.cubic_to(x + w - tr + k * tr, y, x + w, y + tr - k * tr, x + w, y + tr);
    }
    pb.line_to(x + w, y + h - br);
    if br > 0.0 {
        pb.cubic_to(x + w, y + h - br + k * br, x + w - br + k * br, y + h, x + w - br, y + h);
    }
    pb.line_to(x + bl, y + h);
    if bl > 0.0 {
        pb.cubic_to(x + bl - k * bl, y + h, x, y + h - bl + k * bl, x, y + h - bl);
    }
    pb.line_to(x, y + tl);
    if tl > 0.0 {
        pb.cubic_to(x, y + tl - k * tl, x + tl - k * tl, y, x + tl, y);
    }
    pb.close();
    pb.finish()
}

pub fn rrect(r: R, rad: f32) -> Option<Path> {
    rrect4(r, [rad; 4])
}

pub fn solid(c: Color) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(c);
    p.anti_alias = true;
    p
}

pub fn fill(pm: &mut Pixmap, path: &Option<Path>, c: Color, mask: Option<&Mask>) {
    if let Some(path) = path {
        pm.fill_path(path, &solid(c), FillRule::Winding, Transform::identity(), mask);
    }
}

pub fn stroke(pm: &mut Pixmap, path: &Option<Path>, c: Color, w: f32, mask: Option<&Mask>) {
    if let Some(path) = path {
        let st = Stroke { width: w, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() };
        pm.stroke_path(path, &solid(c), &st, Transform::identity(), mask);
    }
}

/// 테두리 상자: 바탕 + 안쪽 테두리(CSS border 와 같은 자리).
pub fn boxed(pm: &mut Pixmap, r: R, rad: f32, bg: Option<Color>, line: Option<(Color, f32)>) {
    if let Some(c) = bg {
        fill(pm, &rrect(r, rad), c, None);
    }
    if let Some((c, w)) = line {
        let half = w / 2.0;
        stroke(pm, &rrect(r.inset(half), (rad - half).max(0.0)), c, w, None);
    }
}

pub fn circle(cx: f32, cy: f32, r: f32) -> Option<Path> {
    PathBuilder::from_circle(cx, cy, r.max(0.1))
}

/// 그림을 원 안에(아바타).
pub fn draw_avatar(pm: &mut Pixmap, img: &Pixmap, cx: f32, cy: f32, r: f32, opacity: f32) {
    let Some(path) = circle(cx, cy, r) else { return };
    let sx = 2.0 * r / img.width().max(1) as f32;
    let sy = 2.0 * r / img.height().max(1) as f32;
    let paint = Paint {
        shader: Pattern::new(
            img.as_ref(),
            SpreadMode::Pad,
            FilterQuality::Bicubic,
            opacity,
            Transform::from_row(sx, 0.0, 0.0, sy, cx - r, cy - r),
        ),
        anti_alias: true,
        ..Paint::default()
    };
    pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// 그림을 칸에 꽉 채워(object-fit: cover) path 모양으로.
pub fn draw_cover(pm: &mut Pixmap, img: &Pixmap, cell: R, path: &Option<Path>) {
    let Some(path) = path else { return };
    let (iw, ih) = (img.width().max(1) as f32, img.height().max(1) as f32);
    let k = (cell.w / iw).max(cell.h / ih);
    let (dw, dh) = (iw * k, ih * k);
    let tx = cell.x + (cell.w - dw) / 2.0;
    let ty = cell.y + (cell.h - dh) / 2.0;
    let paint = Paint {
        shader: Pattern::new(
            img.as_ref(),
            SpreadMode::Pad,
            FilterQuality::Bilinear,
            1.0,
            Transform::from_row(k, 0.0, 0.0, k, tx, ty),
        ),
        anti_alias: true,
        ..Paint::default()
    };
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

/// 선 그리기용 경로 도우미(좌표 = 원점 + 단위 × 값).
pub struct Pen {
    pb: PathBuilder,
    x: f32,
    y: f32,
    u: f32,
}

impl Pen {
    pub fn new(x: f32, y: f32, u: f32) -> Pen {
        Pen { pb: PathBuilder::new(), x, y, u }
    }
    pub fn m(mut self, a: f32, b: f32) -> Pen {
        self.pb.move_to(self.x + a * self.u, self.y + b * self.u);
        self
    }
    pub fn l(mut self, a: f32, b: f32) -> Pen {
        self.pb.line_to(self.x + a * self.u, self.y + b * self.u);
        self
    }
    pub fn q(mut self, a: f32, b: f32, c: f32, d: f32) -> Pen {
        self.pb.quad_to(self.x + a * self.u, self.y + b * self.u, self.x + c * self.u, self.y + d * self.u);
        self
    }
    pub fn c(mut self, a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Pen {
        self.pb.cubic_to(
            self.x + a * self.u,
            self.y + b * self.u,
            self.x + c * self.u,
            self.y + d * self.u,
            self.x + e * self.u,
            self.y + f * self.u,
        );
        self
    }
    pub fn z(mut self) -> Pen {
        self.pb.close();
        self
    }
    pub fn path(self) -> Option<Path> {
        self.pb.finish()
    }
}

/// X 로고(24 단위 경로를 size 로).
pub fn draw_x_logo(pm: &mut Pixmap, x: f32, y: f32, size: f32, c: Color) {
    let u = size / 24.0;
    let p = Pen::new(x, y, u)
        .m(18.9, 2.6)
        .l(22.2, 2.6)
        .l(15.0, 10.8)
        .l(23.5, 22.1)
        .l(16.8, 22.1)
        .l(11.6, 15.3)
        .l(5.6, 22.1)
        .l(1.3, 22.1)
        .l(9.0, 13.3)
        .l(0.7, 2.6)
        .l(7.6, 2.6)
        .l(12.3, 8.8)
        .l(17.9, 2.6)
        .z()
        .m(17.7, 20.3)
        .l(19.6, 20.3)
        .l(7.2, 4.4)
        .l(5.2, 4.4)
        .z()
        .path();
    if let Some(path) = p {
        pm.fill_path(&path, &solid(c), FillRule::EvenOdd, Transform::identity(), None);
    }
}

/// 픽스맵을 다른 픽스맵 위에 — 이동 · 배율(가운데 기준) · 투명도 · 자르기.
pub fn blit(dst: &mut Pixmap, src: &Pixmap, x: f32, y: f32, scale: f32, opacity: f32, mask: Option<&Mask>) {
    if opacity <= 0.003 {
        return;
    }
    let (w, h) = (src.width() as f32, src.height() as f32);
    let tr = if (scale - 1.0).abs() < 1e-4 {
        Transform::from_translate(x, y)
    } else {
        let (cx, cy) = (x + w / 2.0, y + h / 2.0);
        Transform::from_row(scale, 0.0, 0.0, scale, cx - w / 2.0 * scale, cy - h / 2.0 * scale)
    };
    let integral = (scale - 1.0).abs() < 1e-4 && x.fract() == 0.0 && y.fract() == 0.0;
    let paint = PixmapPaint {
        opacity: opacity.clamp(0.0, 1.0),
        quality: if integral { FilterQuality::Nearest } else { FilterQuality::Bilinear },
        ..PixmapPaint::default()
    };
    dst.draw_pixmap(0, 0, src.as_ref(), &paint, tr, mask);
}

// ---------------------------------------------------------------------------
// 글자 — GDI 로 흰 글자(검정 바탕)를 그려 덮임 정도를 받고, 그 정도만큼 색을 섞는다.

/// 한 덩어리 글자.
#[derive(Clone, Debug)]
pub struct Txt {
    pub font: HFONT,
    pub text: String,
    pub rect: R,
    pub color: Color,
    pub fmt: DRAW_TEXT_FORMAT,
    /// 이 안으로 자른다(없으면 rect 와 그림 전체)
    pub clip: Option<R>,
}

/// 재사용하는 흑백 DIB.
pub struct TextPainter {
    dc: HDC,
    bmp: Option<HBITMAP>,
    bits: *mut u8,
    w: i32,
    h: i32,
}

impl TextPainter {
    pub fn new() -> TextPainter {
        TextPainter { dc: unsafe { CreateCompatibleDC(None) }, bmp: None, bits: null_mut(), w: 0, h: 0 }
    }

    fn ensure(&mut self, w: i32, h: i32) -> bool {
        if self.bmp.is_some() && self.w >= w && self.h >= h {
            return true;
        }
        let (w, h) = (w.max(self.w), h.max(self.h));
        unsafe {
            let screen = GetDC(None);
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
            let bmp = CreateDIBSection(Some(screen), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok();
            ReleaseDC(None, screen);
            let Some(bmp) = bmp else { return false };
            SelectObject(self.dc, HGDIOBJ(bmp.0));
            if let Some(old) = self.bmp.replace(bmp) {
                let _ = DeleteObject(HGDIOBJ(old.0));
            }
            self.bits = bits as *mut u8;
            self.w = w;
            self.h = h;
            SetBkMode(self.dc, TRANSPARENT);
            SetTextColor(self.dc, COLORREF(0x00ff_ffff));
        }
        true
    }

    /// 글자들을 pm 위에(pm 좌표).
    pub fn draw(&mut self, pm: &mut Pixmap, txt: &[Txt]) {
        let (pw, ph) = (pm.width() as i32, pm.height() as i32);
        if txt.is_empty() || !self.ensure(pw, ph) {
            return;
        }
        let stride = self.w as usize * 4;
        let whole = R::new(0.0, 0.0, pw as f32, ph as f32);
        for t in txt {
            if t.text.is_empty() {
                continue;
            }
            let area = match t.clip {
                Some(c) => t.rect.intersect(&c).and_then(|r| r.intersect(&whole)),
                None => t.rect.intersect(&whole),
            };
            let Some(area) = area else { continue };
            let a = area.rc();
            let (x0, y0) = (a.left.max(0), a.top.max(0));
            let (x1, y1) = (a.right.min(pw), a.bottom.min(ph));
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            unsafe {
                let buf = std::slice::from_raw_parts_mut(self.bits, stride * self.h as usize);
                for y in y0..y1 {
                    let row = y as usize * stride;
                    buf[row + x0 as usize * 4..row + x1 as usize * 4].fill(0);
                }
                let old = SelectObject(self.dc, HGDIOBJ(t.font.0));
                let _ = IntersectClipRect(self.dc, x0, y0, x1, y1);
                let mut wb: Vec<u16> = t.text.encode_utf16().collect();
                let mut r = t.rect.rc();
                DrawTextW(self.dc, &mut wb, &mut r, t.fmt | DT_NOPREFIX);
                SelectClipRgn(self.dc, None);
                SelectObject(self.dc, old);
                // 섞기: src(색 × 덮임) over dst
                let (cr, cg, cb, ca) = (t.color.red(), t.color.green(), t.color.blue(), t.color.alpha());
                let dst = pm.data_mut();
                for y in y0..y1 {
                    let row = y as usize * stride;
                    for x in x0..x1 {
                        let cov = buf[row + x as usize * 4 + 1] as f32 / 255.0;
                        if cov <= 0.0 {
                            continue;
                        }
                        // 흑백 안티에일리어싱은 가늘게 나와 조금 굵힌다(브라우저 글자와 비슷하게)
                        let a = (1.0 - (1.0 - cov).powf(1.3)) * ca;
                        let i = (y as usize * pw as usize + x as usize) * 4;
                        let inv = 1.0 - a;
                        dst[i] = (cr * a * 255.0 + dst[i] as f32 * inv).round() as u8;
                        dst[i + 1] = (cg * a * 255.0 + dst[i + 1] as f32 * inv).round() as u8;
                        dst[i + 2] = (cb * a * 255.0 + dst[i + 2] as f32 * inv).round() as u8;
                        dst[i + 3] = (a * 255.0 + dst[i + 3] as f32 * inv).round() as u8;
                    }
                }
            }
        }
    }
}

impl Drop for TextPainter {
    fn drop(&mut self) {
        unsafe {
            if let Some(b) = self.bmp.take() {
                let _ = DeleteObject(HGDIOBJ(b.0));
            }
            let _ = DeleteDC(self.dc);
        }
    }
}

// ---------------------------------------------------------------------------
// 말풍선 틀

pub struct Frame {
    pub w: i32,
    pub h: i32,
    /// 말풍선 왼쪽 위(창 좌표)
    pub bx: f32,
    pub by: f32,
    pub bh: f32,
    /// 꼬리 끝 x(창 좌표)
    pub tail_x: f32,
}

impl Frame {
    pub fn new(s: f32, bubble_h: f32, tail_x_in_bubble: f32) -> Frame {
        let sh = SHADOW * s;
        let w = (BUBBLE_W * s + 2.0 * sh).ceil() as i32;
        let h = (sh + bubble_h + TAIL_H * s + 2.0).ceil() as i32;
        Frame { w, h, bx: sh, by: sh, bh: bubble_h, tail_x: sh + tail_x_in_bubble }
    }
    pub fn tail_tip(&self, s: f32) -> (f32, f32) {
        (self.tail_x, self.by + self.bh + TAIL_H * s)
    }
}

/// 말풍선 바탕(그림자 · 172deg 그라데이션 · 꼬리). 비싸므로 크기 · 꼬리 · 색이 같으면 다시 쓴다.
pub fn background(fr: &Frame, pal: &Palette, s: f32) -> Option<Pixmap> {
    let mut pm = Pixmap::new(fr.w as u32, fr.h as u32)?;
    let bw = BUBBLE_W * s;
    let br = R::new(fr.bx, fr.by, bw, fr.bh);

    // 그림자 — 바깥으로 갈수록 옅어지는 겹
    for i in 0..12 {
        let k = i as f32;
        let r = R::new(br.x - k * s, br.y + 8.0 * s - k * s, br.w + 2.0 * k * s, br.h + 2.0 * k * s - 4.0 * s);
        fill(&mut pm, &rrect(r, (RADIUS + k) * s), BLACK.color(0.022), None);
    }

    let a = 172f32.to_radians();
    let (dx, dy) = (a.sin(), -a.cos());
    let len = (br.w * dx).abs() + (br.h * dy).abs();
    let (cx, cy) = (br.x + br.w / 2.0, br.y + br.h / 2.0);
    let top = pal.m.mix(WHITE, 0.88);
    let bot = pal.m.mix(BLACK, 0.90);
    let shader = LinearGradient::new(
        Point::from_xy(cx - dx * len / 2.0, cy - dy * len / 2.0),
        Point::from_xy(cx + dx * len / 2.0, cy + dy * len / 2.0),
        vec![
            GradientStop::new(0.0, top.color(1.0)),
            GradientStop::new(0.48, pal.m.color(1.0)),
            GradientStop::new(1.0, bot.color(1.0)),
        ],
        SpreadMode::Pad,
        Transform::identity(),
    )
    .unwrap_or(Shader::SolidColor(pal.m.color(1.0)));
    let paint = Paint { shader, anti_alias: true, ..Paint::default() };

    let (tx, ty) = fr.tail_tip(s);
    let base = br.y + br.h - 2.0 * s;
    let half = TAIL_HALF * s;
    let mut pb = PathBuilder::new();
    pb.move_to(tx - half, base);
    pb.quad_to(tx - half * 0.25, base + (ty - base) * 0.35, tx, ty);
    pb.quad_to(tx + half * 0.35, base + (ty - base) * 0.3, tx + half, base);
    pb.close();
    if let Some(tail) = pb.finish() {
        pm.fill_path(&tail, &paint, FillRule::Winding, Transform::identity(), None);
    }
    if let Some(body) = rrect(br, RADIUS * s) {
        pm.fill_path(&body, &paint, FillRule::Winding, Transform::identity(), None);
        stroke(&mut pm, &Some(body), pal.mi_a(0.12), s, None);
    }
    Some(pm)
}

/// 사각형 자르기 마스크(+ 둥근 상자와 겹친 곳만).
pub fn clip_mask(w: u32, h: u32, rect: R, also: Option<(R, f32)>) -> Option<Mask> {
    let mut m = Mask::new(w, h)?;
    let mut pb = PathBuilder::new();
    pb.push_rect(rect.rect()?);
    m.fill_path(&pb.finish()?, FillRule::Winding, false, Transform::identity());
    if let Some((r, rad)) = also {
        if let Some(p) = rrect(r, rad) {
            m.intersect_path(&p, FillRule::Winding, true, Transform::identity());
        }
    }
    Some(m)
}

/// 피드 위 · 아래 가장자리를 바탕 쪽으로 서서히(mask-image 선형 그라데이션).
pub fn fade_rows(pm: &mut Pixmap, bg: &Pixmap, feed: R, top: f32, bot: f32) {
    let w = pm.width() as usize;
    let ph = pm.height() as usize;
    let x0 = feed.x.max(0.0) as usize;
    let x1 = ((feed.x + feed.w) as usize).min(w);
    let y0 = feed.y.max(0.0);
    let y1 = (feed.y + feed.h).min(ph as f32);
    let rows = move |a: f32, b: f32| (a.floor().max(0.0) as usize)..(b.ceil().max(0.0) as usize).min(ph);
    let src = bg.data();
    let apply = |dst: &mut [u8], y: usize, t: f32| {
        let t = t.clamp(0.0, 1.0);
        for x in x0..x1 {
            let i = (y * w + x) * 4;
            for k in 0..4 {
                dst[i + k] = (src[i + k] as f32 + (dst[i + k] as f32 - src[i + k] as f32) * t).round() as u8;
            }
        }
    };
    let dst = pm.data_mut();
    if top > 0.0 {
        for y in rows(y0, y0 + top) {
            apply(dst, y, (y as f32 + 0.5 - y0) / top);
        }
    }
    if bot > 0.0 {
        for y in rows(y1 - bot, y1) {
            apply(dst, y, (y1 - (y as f32 + 0.5)) / bot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ink_follows_contrast() {
        assert_eq!(Palette::new(Some(Rgb(255.0, 170.0, 204.0))).mi, INK_DARK);
        assert_eq!(Palette::new(Some(Rgb(40.0, 40.0, 90.0))).mi, WHITE);
        assert_eq!(Palette::new(None).mi, WHITE);
    }

    #[test]
    fn tokens_keep_spaces() {
        assert_eq!(tokens("가 나  다"), vec![(0, "가 "), (4, "나  "), (9, "다")]);
        assert!(tokens("").is_empty());
    }

    #[test]
    fn links_found() {
        let t = "見てね https://youtu.be/x #みるみやこ @yuno さん";
        let r: Vec<&str> = link_ranges(t).into_iter().map(|(a, b)| &t[a..b]).collect();
        assert_eq!(r, vec!["https://youtu.be/x", "#みるみやこ", "@yuno"]);
    }

    #[test]
    fn wrap_keeps_blank_lines_and_breaks_long() {
        let me = Measurer::new();
        let f = Fonts::new(1.0);
        let font = f.get(15.0, false);
        let lines = me.wrap(font, "가나다\n\n라마", 1000.0);
        assert_eq!(lines, vec!["가나다", "", "라마"]);
        // 띄어쓰기 없는 긴 글은 글자 단위로 나뉜다
        let long = "あ".repeat(60);
        let ls = me.wrap(font, &long, 200.0);
        assert!(ls.len() > 1);
        assert_eq!(ls.concat(), long);
    }

    #[test]
    fn avatars_load() {
        for k in ["arale", "yuno", "nonoka", "ritsu", "miyako"] {
            assert!(load_avatar(k).expect(k).color.is_some());
        }
    }
}
