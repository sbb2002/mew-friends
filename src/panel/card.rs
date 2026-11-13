//! 글 카드 두 면 — 번역 면(.tws-ko) · 원문 면(X 카드). 원본 tweets.js `_article` · tweetlogic `XM` 치수.
//!
//! 두 면은 같은 칸에 겹친다(높이 = 큰 쪽). 번역 면의 본문 · 인용 본문 칸은 원문 높이만큼 자리를 잡아(min-height)
//! 넘겨도 사진 · 인용이 같은 자리에 온다. 면마다 그릴 것(Prim)과 누를 곳(hits)을 미리 정해 두고, 그림은 면 하나를
//! 픽스맵 한 장으로 그려 캐시한다(panel.rs).

use std::rc::Rc;

use chrono::{DateTime, FixedOffset, Timelike, Datelike, Utc};
use tiny_skia::{Color, Pixmap};
use windows::Win32::Graphics::Gdi::{
    DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE, DT_VCENTER,
};

use super::view::{
    self, Avatar, Fonts, Measurer, Palette, Pen, R, Rgb, TextPainter, Txt, WHITE, BLACK, xc,
};
use super::{PanelTweet, reply_label, time_label};
use crate::net::{self, Fx};

/// 누를 수 있는 곳.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Chip(String),
    /// 머리 줄 「번역 · 원문」 탭(0 = 번역, 1 = 원문)
    Seg(u8),
    Live,
    Copy(String),
    Open(String),
    Fab,
    Video { id: String, quote: bool, thumb: String, idx: usize, url: String },
    /// 카드 · 점 줄(밀기 손잡이 — 눌러도 아무 일 없음)
    Track(String),
}

/// 그리기 맥락.
pub struct Ctx<'a> {
    pub s: f32,
    pub me: &'a Measurer,
    pub fonts: &'a Fonts,
    pub pal: &'a Palette,
    pub avatars: &'a [Avatar],
}

impl Ctx<'_> {
    fn p(&self, v: f32) -> f32 {
        v * self.s
    }
    fn w(&self, size: f32, bold: bool, t: &str) -> f32 {
        self.me.width(self.fonts.get(size, bold), t)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Reply,
    XLogo,
    Heart,
    Bubble,
    Link,
}

#[derive(Clone, Debug)]
enum AvSrc {
    Member(String),
    Url(String, String),
}

#[derive(Clone, Debug)]
enum Prim {
    Box { r: R, rad: f32, bg: Option<Color>, line: Option<(Color, f32)> },
    /// 올렸을 때만 보이는 바탕
    Hover { r: R, rad: f32, act: Act, color: Color },
    Circle { cx: f32, cy: f32, r: f32, color: Color },
    Avatar { src: AvSrc, cx: f32, cy: f32, r: f32 },
    Cell { r: R, corners: [f32; 4], url: String, video: bool, gif: bool, more: usize, x: bool, bg: Color },
    Icon { kind: Icon, x: f32, y: f32, size: f32, color: Color, hover: Option<(Act, Color)> },
    Text { size: f32, bold: bool, text: String, r: R, color: Color, fmt: DRAW_TEXT_FORMAT, hover: Option<(Act, Color)> },
    /// URL 복사 글자(복사됨 · 복사 실패로 바뀐다)
    CopyLabel { r: R, id: String, color: Color },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Ko,
    X,
}

/// 면 하나의 배치(면 안 좌표).
#[derive(Clone, Debug)]
pub struct FaceLay {
    pub kind: Kind,
    pub h: f32,
    prims: Vec<Prim>,
    pub hits: Vec<(R, Act)>,
    /// 이 면이 쓰는 그림 주소(받기 · 캐시 서명용)
    urls: Vec<String>,
}

/// 카드(글 하나) 배치 — 피드 내용 좌표.
#[derive(Clone, Debug)]
pub struct CardLay {
    pub tw: PanelTweet,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub track_h: f32,
    pub in_chain: bool,
    pub root: bool,
    pub ko: FaceLay,
    pub xf: FaceLay,
}

impl CardLay {
    /// 글 하나 높이 = 칸 + 4 + 점 줄 20.
    pub fn h(&self, s: f32) -> f32 {
        self.track_h + 24.0 * s
    }
    pub fn face(&self, f: u8) -> &FaceLay {
        if f == 0 { &self.ko } else { &self.xf }
    }
}

const LEFT1: DRAW_TEXT_FORMAT = DRAW_TEXT_FORMAT(DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0);
const LEFT1E: DRAW_TEXT_FORMAT = DRAW_TEXT_FORMAT(DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0);
const CENTER1: DRAW_TEXT_FORMAT = DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0);

fn text(size: f32, bold: bool, t: impl Into<String>, r: R, color: Color, fmt: DRAW_TEXT_FORMAT) -> Prim {
    Prim::Text { size, bold, text: t.into(), r, color, fmt, hover: None }
}

/// 1칸 비율(가로 ÷ 세로) — 세로로 길면 정사각형, 모르면 16:9 (tweetlogic singleRatio).
fn single_ratio(url: &str) -> f32 {
    match net::image_ready(url) {
        Some(p) if p.width() > 0 && p.height() > 0 => (p.width() as f32 / p.height() as f32).max(1.0),
        _ => 16.0 / 9.0,
    }
}

/// 미디어 격자(.tws-md). 반환 = 높이. 2~4칸 = 16:9 묶음, 1칸 = 원래 비율. 칸 사이 2, 넷째 칸에 +N.
#[allow(clippy::too_many_arguments)]
fn grid(
    c: &Ctx,
    prims: &mut Vec<Prim>,
    hits: &mut Vec<(R, Act)>,
    urls: &mut Vec<String>,
    items: &[String],
    x: f32,
    y: f32,
    w: f32,
    border: f32,
    radius: [f32; 4],
    xstyle: bool,
    tw: &PanelTweet,
    quote: bool,
) -> f32 {
    let n = items.len();
    if n == 0 || w <= 0.0 {
        return 0.0;
    }
    let shown = n.min(4);
    let more = n.saturating_sub(4);
    let inner = w - 2.0 * border;
    let ratio = if shown == 1 { single_ratio(&items[0]) } else { 16.0 / 9.0 };
    let ih = (inner / ratio).round();
    let h = ih + 2.0 * border;
    let gap = c.p(2.0);
    let (line, bgc, cellc) = if xstyle {
        (xc::LINE.color(1.0), xc::CELL.color(1.0), xc::CELL.color(1.0))
    } else {
        (c.pal.mi_a(0.26), c.pal.mi_a(0.10), c.pal.mi_a(0.12))
    };
    let outer = R::new(x, y, w, h);
    prims.push(Prim::Box {
        r: outer,
        rad: radius[0].max(radius[2]),
        bg: Some(bgc),
        line: (border > 0.0).then_some((line, border)),
    });
    let ir = R::new(x + border, y + border, inner, ih);
    let rin = radius.map(|r| (r - border).max(0.0));
    let (hw, hh) = ((inner - gap) / 2.0, (ih - gap) / 2.0);
    let cells: Vec<(R, [f32; 4])> = match shown {
        1 => vec![(ir, rin)],
        2 => vec![
            (R::new(ir.x, ir.y, hw, ih), [rin[0], 0.0, 0.0, rin[3]]),
            (R::new(ir.x + hw + gap, ir.y, hw, ih), [0.0, rin[1], rin[2], 0.0]),
        ],
        3 => vec![
            (R::new(ir.x, ir.y, hw, ih), [rin[0], 0.0, 0.0, rin[3]]),
            (R::new(ir.x + hw + gap, ir.y, hw, hh), [0.0, rin[1], 0.0, 0.0]),
            (R::new(ir.x + hw + gap, ir.y + hh + gap, hw, hh), [0.0, 0.0, rin[2], 0.0]),
        ],
        _ => vec![
            (R::new(ir.x, ir.y, hw, hh), [rin[0], 0.0, 0.0, 0.0]),
            (R::new(ir.x + hw + gap, ir.y, hw, hh), [0.0, rin[1], 0.0, 0.0]),
            (R::new(ir.x, ir.y + hh + gap, hw, hh), [0.0, 0.0, 0.0, rin[3]]),
            (R::new(ir.x + hw + gap, ir.y + hh + gap, hw, hh), [0.0, 0.0, rin[2], 0.0]),
        ],
    };
    let mut vidx = 0;
    for (i, (r, corners)) in cells.into_iter().enumerate() {
        let url = items[i].clone();
        let video = net::is_video_url(&url);
        urls.push(url.clone());
        prims.push(Prim::Cell {
            r,
            corners,
            url: url.clone(),
            video,
            // GIF 썸네일 주소(tweet_video_thumb) — 원문 면 왼쪽 아래 「GIF」
            gif: xstyle && url.contains("/tweet_video_thumb/"),
            more: if i == 3 { more } else { 0 },
            x: xstyle,
            bg: cellc,
        });
        if video {
            hits.push((r, Act::Video { id: tw.id.clone(), quote, thumb: url, idx: vidx, url: tw.url.clone().unwrap_or_default() }));
            vidx += 1;
        } else if xstyle {
            if let Some(u) = &tw.url {
                hits.push((r, Act::Open(u.clone())));
            }
        }
    }
    h
}

fn kst(ms: i64) -> Option<DateTime<FixedOffset>> {
    DateTime::<Utc>::from_timestamp_millis(ms).map(|d| d.with_timezone(&FixedOffset::east_opt(9 * 3600).unwrap()))
}

/// X 카드 시각 줄: 「오후 7:55 · 2026년 9월 6일」
pub fn x_time(ms: i64) -> String {
    let Some(t) = kst(ms) else { return String::new() };
    let (ap, h12) = if t.hour() < 12 { ("오전", t.hour()) } else { ("오후", t.hour() - 12) };
    let h12 = if h12 == 0 { 12 } else { h12 };
    format!("{ap} {h12}:{:02} · {}년 {}월 {}일", t.minute(), t.year(), t.month(), t.day())
}

/// 원문 면(X 카드). 반환 = (면, 본문 높이, 인용 본문 높이) — 번역 면이 위치를 맞추는 데 쓴다.
pub fn layout_x(c: &Ctx, t: &PanelTweet, fx: Option<&Fx>, w: f32) -> (FaceLay, f32, f32) {
    let p = |v: f32| c.p(v);
    let mut prims = Vec::new();
    let mut hits = Vec::new();
    let mut urls = Vec::new();
    let x0 = p(17.0);
    let cw = w - p(34.0);
    let mut y = p(13.0);
    let url = t.url.clone().unwrap_or_default();
    let handle = fx.map(|f| f.handle.clone()).filter(|h| !h.is_empty()).unwrap_or_else(|| t.handle.clone());

    // 머리 줄 48: 얼굴 40 · 이름 · @핸들 · X 로고
    let src = match fx.and_then(|f| f.avatar.clone()) {
        Some(u) => {
            urls.push(u.clone());
            AvSrc::Url(u, t.member.clone())
        }
        None => AvSrc::Member(t.member.clone()),
    };
    prims.push(Prim::Avatar { src, cx: x0 + p(20.0), cy: y + p(24.0), r: p(20.0) });
    let name = fx.map(|f| f.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| t.x_name.clone());
    let id_x = x0 + p(52.0);
    let id_w = w - p(17.0 + 30.0) - id_x;
    prims.push(text(15.0, true, name, R::new(id_x, y + p(4.0), id_w, p(20.0)), xc::TEXT.color(1.0), LEFT1E));
    if !handle.is_empty() {
        prims.push(text(15.0, false, format!("@{handle}"), R::new(id_x, y + p(24.0), id_w, p(20.0)), xc::SUB.color(1.0), LEFT1E));
    }
    let lx = w - p(17.0 + 22.0);
    prims.push(Prim::Icon { kind: Icon::XLogo, x: lx, y: y + p(2.0), size: p(22.0), color: xc::TEXT.color(1.0), hover: None });
    if !url.is_empty() {
        hits.push((R::new(lx - p(4.0), y, p(30.0), p(28.0)), Act::Open(url.clone())));
    }
    y += p(48.0 + 12.0);

    // 답글 줄 24: 「@핸들 님에게 보내는 답글」
    if t.reply_to.is_some() {
        let at = format!("@{handle}");
        let aw = c.w(15.0, false, &at);
        prims.push(text(15.0, false, at, R::new(x0, y, aw + p(2.0), p(24.0)), xc::BLUE.color(1.0), LEFT1));
        prims.push(text(15.0, false, " 님에게 보내는 답글", R::new(x0 + aw, y, cw - aw, p(24.0)), xc::SUB.color(1.0), LEFT1E));
        y += p(24.0);
    }

    // 본문 20px / 24px (주소 · #태그 · @멘션은 파랗게)
    let text_h = rich_lines(c, &mut prims, &t.orig, 20.0, 24.0, x0, y, cw, xc::TEXT, xc::BLUE);
    y += text_h;

    if !t.media.is_empty() {
        y += p(12.0);
        y += grid(c, &mut prims, &mut hits, &mut urls, &t.media, x0, y, cw, p(1.0), [p(16.0); 4], true, t, false);
    }

    let mut qtext_h = 0.0;
    if let Some(q) = &t.quote {
        y += p(12.0);
        let qtop = y;
        let idx = prims.len();
        let qx = x0 + p(13.0);
        let qw = cw - p(26.0);
        let mut y1 = y + p(13.0);
        // 이름 줄 26: 얼굴 20 · 이름 · @핸들 (fxtwitter 를 받기 전엔 「인용한 글」)
        match fx.and_then(|f| f.quote_name.clone()) {
            Some(qn) => {
                if let Some(u) = fx.and_then(|f| f.quote_avatar.clone()) {
                    urls.push(u.clone());
                    prims.push(Prim::Avatar { src: AvSrc::Url(u, String::new()), cx: qx + p(10.0), cy: y1 + p(11.0), r: p(10.0) });
                }
                let nx = qx + p(26.0);
                let nw = c.w(15.0, true, &qn).min(qw - p(26.0));
                prims.push(text(15.0, true, qn, R::new(nx, y1, nw + p(1.0), p(22.0)), xc::TEXT.color(1.0), LEFT1E));
                if let Some(h) = fx.and_then(|f| f.quote_handle.clone()) {
                    let hx = nx + nw + p(4.0);
                    prims.push(text(15.0, false, format!("@{h}"), R::new(hx, y1, (qx + qw - hx).max(0.0), p(22.0)), xc::SUB.color(1.0), LEFT1E));
                }
            }
            None => prims.push(text(15.0, true, "인용한 글", R::new(qx, y1, qw, p(22.0)), xc::TEXT.color(1.0), LEFT1)),
        }
        y1 += p(26.0);
        qtext_h = rich_lines(c, &mut prims, q.text.as_deref().unwrap_or(""), 15.0, 20.0, qx, y1, qw, xc::TEXT, xc::BLUE);
        y1 += qtext_h + p(12.0);
        if !q.media.is_empty() {
            let r = p(15.0);
            y1 += grid(c, &mut prims, &mut hits, &mut urls, &q.media, x0 + p(1.0), y1, cw - p(2.0), 0.0, [0.0, 0.0, r, r], true, t, true);
        }
        y1 += p(1.0);
        prims.insert(idx, Prim::Box { r: R::new(x0, qtop, cw, y1 - qtop), rad: p(16.0), bg: None, line: Some((xc::LINE.color(1.0), p(1.0))) });
        y = y1;
    }

    // 시각 줄
    y += p(8.0);
    let ms = fx.and_then(|f| f.created_ms).or(t.ms);
    prims.push(text(15.0, false, ms.map(x_time).unwrap_or_default(), R::new(x0, y, cw, p(20.0)), xc::SUB.color(1.0), LEFT1));
    y += p(20.0);

    // 아래 100: 구분선 · 반응 줄 · 「답글 읽기」 알약
    prims.push(Prim::Box { r: R::new(x0, y + p(11.0), cw, p(1.0)), rad: 0.0, bg: Some(xc::LINE.color(1.0)), line: None });
    let ay = y + p(17.0);
    let mut ax = x0 - p(6.0);
    let likes = fx.map(|f| f.likes);
    let reps = fx.map(|f| f.replies);
    let actions = [
        (Icon::Heart, likes.map(compact).unwrap_or_else(|| "좋아요".into()), Act::Open(url.clone())),
        (Icon::Bubble, "답글".to_string(), Act::Open(url.clone())),
        (Icon::Link, "링크 복사".to_string(), Act::Copy(t.id.clone())),
    ];
    for (icon, label, act) in actions {
        let tw = c.w(13.0, true, &label);
        let bw = p(6.0 + 20.0 + 4.0 + 8.0) + tw;
        let r = R::new(ax, ay, bw, p(32.0));
        prims.push(Prim::Hover { r, rad: p(16.0), act: act.clone(), color: xc::HOVER.color(1.0) });
        let col = if icon == Icon::Heart { Rgb::hex(0xf91880) } else { xc::SUB };
        prims.push(Prim::Icon { kind: icon, x: ax + p(6.0), y: ay + p(6.0), size: p(20.0), color: col.color(1.0), hover: None });
        prims.push(text(13.0, true, label, R::new(ax + p(30.0), ay, tw + p(4.0), p(32.0)), xc::SUB.color(1.0), LEFT1));
        if !url.is_empty() {
            hits.push((r, act));
        }
        ax += bw + p(4.0);
    }
    let py = y + p(57.0);
    let pill = R::new(x0, py, cw, p(36.0));
    let plabel = match reps {
        Some(n) if n > 0 => format!("답글 {}개 읽기", compact(n)),
        _ => "X 에서 보기".to_string(),
    };
    prims.push(Prim::Hover { r: pill, rad: p(18.0), act: Act::Open(url.clone()), color: Rgb::hex(0x031018).color(1.0) });
    prims.push(Prim::Box { r: pill, rad: p(18.0), bg: None, line: Some((xc::PILL.color(1.0), p(1.0))) });
    prims.push(text(15.0, true, plabel, pill, xc::BLUE.color(1.0), CENTER1));
    if !url.is_empty() {
        hits.push((pill, Act::Open(url)));
    }
    y += p(100.0);

    (FaceLay { kind: Kind::X, h: y.round(), prims, hits, urls }, text_h, qtext_h)
}

/// 639 → 「639」, 12345 → 「1.2만」
fn compact(n: u64) -> String {
    if n >= 10_000 {
        let v = n as f64 / 10_000.0;
        if v >= 100.0 { format!("{}만", v.round()) } else { format!("{:.1}만", v).replace(".0만", "만") }
    } else {
        n.to_string()
    }
}

/// 여러 줄 글자(링크 낱말은 다른 색). 반환 = 높이.
#[allow(clippy::too_many_arguments)]
fn rich_lines(c: &Ctx, prims: &mut Vec<Prim>, s: &str, size: f32, lh: f32, x: f32, y: f32, w: f32, col: Rgb, link: Rgb) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    let f = c.fonts.get(size, false);
    let lines = c.me.wrap_idx(f, s, w);
    let links = view::link_ranges(s);
    let lh = c.p(lh);
    for (i, (a, b)) in lines.iter().copied().enumerate() {
        let ly = y + lh * i as f32;
        // 이 줄을 링크 / 보통 조각으로
        let mut cuts = vec![a, b];
        for (la, lb) in &links {
            for v in [*la, *lb] {
                if v > a && v < b {
                    cuts.push(v);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for win in cuts.windows(2) {
            let (sa, sb) = (win[0], win[1]);
            if sa >= sb {
                continue;
            }
            let is_link = links.iter().any(|(la, lb)| sa >= *la && sb <= *lb);
            let off = c.me.width(f, &s[a..sa]);
            let segw = c.me.width(f, &s[sa..sb]);
            prims.push(text(
                size,
                false,
                &s[sa..sb],
                R::new(x + off, ly, segw + c.p(2.0), lh),
                if is_link { link.color(1.0) } else { col.color(1.0) },
                LEFT1,
            ));
        }
    }
    lh * lines.len() as f32
}

/// 번역 면(.tws-ko). x_text_h · x_qtext_h = 원문 면 본문 높이(번역 본문 칸 min-height).
pub fn layout_ko(c: &Ctx, t: &PanelTweet, prev: Option<&PanelTweet>, w: f32, x_text_h: f32, x_qtext_h: f32) -> FaceLay {
    let p = |v: f32| c.p(v);
    let pal = c.pal;
    let sub = pal.sub().color(1.0);
    let mi = pal.mi.color(1.0);
    let mut prims = Vec::new();
    let mut hits = Vec::new();
    let mut urls = Vec::new();
    let x0 = p(17.0);
    let cw = w - p(34.0);
    let mut y = p(13.0);

    // 머리 줄 48: 얼굴 40(옆 4) · 이름 · 핸들 · URL 복사
    prims.push(Prim::Circle { cx: x0 + p(24.0), cy: y + p(24.0), r: p(21.5), color: pal.mi_a(0.4) });
    prims.push(Prim::Avatar { src: AvSrc::Member(t.member.clone()), cx: x0 + p(24.0), cy: y + p(24.0), r: p(20.0) });
    let copy_w = c.w(11.0, true, "URL 복사").max(c.w(11.0, true, "복사 실패")) + p(14.0);
    let copy = R::new(w - p(17.0) - copy_w, y + p(14.0), copy_w, p(20.0));
    let id_x = x0 + p(58.0);
    let id_w = copy.x - p(10.0) - id_x;
    prims.push(text(14.5, true, t.name_ko.clone(), R::new(id_x, y + p(4.0), id_w, p(20.0)), mi, LEFT1E));
    if !t.handle.is_empty() {
        prims.push(text(12.5, false, format!("@{}", t.handle), R::new(id_x, y + p(24.0), id_w, p(20.0)), sub, LEFT1E));
    }
    let copy_act = Act::Copy(t.id.clone());
    prims.push(Prim::Hover { r: copy, rad: p(6.0), act: copy_act.clone(), color: pal.mi_a(0.14) });
    prims.push(Prim::Box { r: copy, rad: p(6.0), bg: None, line: Some((pal.mi_a(0.26), p(1.0))) });
    prims.push(Prim::CopyLabel { r: copy, id: t.id.clone(), color: mi });
    hits.push((copy.inset(-p(6.0)), copy_act));
    y += p(48.0);

    // 「이어서」 줄 24
    let mt = if t.reply_to.is_some() {
        y += p(12.0);
        prims.push(Prim::Icon { kind: Icon::Reply, x: x0, y: y + p(5.5), size: p(13.0), color: sub, hover: None });
        prims.push(text(12.5, false, reply_label(t, prev), R::new(x0 + p(17.0), y, cw - p(17.0), p(24.0)), sub, LEFT1E));
        y += p(24.0);
        0.0
    } else {
        p(12.0)
    };

    // 본문 15.5px / 1.6 — 원문 본문 높이만큼은 자리를 잡는다
    y += mt;
    let lines = c.me.wrap(c.fonts.get(15.5, false), &t.text, cw);
    let lh = p(15.5 * 1.6);
    for (i, l) in lines.iter().enumerate() {
        prims.push(text(15.5, false, l.clone(), R::new(x0, y + lh * i as f32, cw + p(2.0), lh), mi, LEFT1));
    }
    y += (lh * lines.len() as f32).max(x_text_h);

    if !t.media.is_empty() {
        y += p(12.0);
        y += grid(c, &mut prims, &mut hits, &mut urls, &t.media, x0, y, cw, p(1.0), [p(12.0); 4], false, t, false);
    }

    if let Some(q) = &t.quote {
        y += p(12.0);
        let qtop = y;
        let idx = prims.len();
        let qx = x0 + p(13.0);
        let qw = cw - p(26.0);
        let mut y1 = y + p(13.0);
        prims.push(text(12.0, true, "인용한 글", R::new(qx, y1, qw, p(20.0)), mi, LEFT1));
        y1 += p(26.0);
        let qt = q.text_ko.as_deref().filter(|s| !s.trim().is_empty()).unwrap_or(super::PENDING_TEXT);
        let qlines = c.me.wrap(c.fonts.get(14.0, false), qt, qw);
        for (i, l) in qlines.iter().enumerate() {
            prims.push(text(14.0, false, l.clone(), R::new(qx, y1 + p(20.0) * i as f32, qw + p(2.0), p(20.0)), mi, LEFT1));
        }
        y1 += (p(20.0) * qlines.len() as f32).max(x_qtext_h) + p(12.0);
        if !q.media.is_empty() {
            let r = p(11.0);
            y1 += grid(c, &mut prims, &mut hits, &mut urls, &q.media, x0 + p(1.0), y1, cw - p(2.0), 0.0, [0.0, 0.0, r, r], false, t, true);
        }
        y1 += p(1.0);
        prims.insert(idx, Prim::Box { r: R::new(x0, qtop, cw, y1 - qtop), rad: p(12.0), bg: None, line: Some((pal.mi_a(0.26), p(1.0))) });
        y = y1;
    }

    // 바닥 줄: 시각 · 원문
    y += p(8.0);
    let time = time_label(t);
    prims.push(text(12.5, false, time.clone(), R::new(x0, y, p(220.0), p(20.0)), sub, LEFT1));
    if let Some(url) = &t.url {
        let ox = x0 + c.w(12.5, false, &time) + if time.is_empty() { 0.0 } else { p(8.0) };
        let ow = p(6.0 + 12.0 + 4.0 + 6.0) + c.w(12.0, true, "원문");
        let r = R::new(ox, y, ow, p(20.0));
        let act = Act::Open(url.clone());
        prims.push(Prim::Hover { r, rad: p(6.0), act: act.clone(), color: pal.mi_a(0.14) });
        prims.push(Prim::Icon { kind: Icon::XLogo, x: ox + p(6.0), y: y + p(4.0), size: p(12.0), color: sub, hover: Some((act.clone(), mi)) });
        prims.push(Prim::Text {
            size: 12.0,
            bold: true,
            text: "원문".into(),
            r: R::new(ox + p(22.0), y, ow - p(22.0), p(20.0)),
            color: sub,
            fmt: LEFT1,
            hover: Some((act.clone(), mi)),
        });
        hits.push((r, act));
    }
    y += p(20.0 + 13.0);

    FaceLay { kind: Kind::Ko, h: y.round(), prims, hits, urls }
}

/// 카드 하나(두 면) 배치.
#[allow(clippy::too_many_arguments)]
pub fn layout_card(c: &Ctx, t: &PanelTweet, prev: Option<&PanelTweet>, x: f32, y: f32, w: f32, in_chain: bool, root: bool) -> CardLay {
    let fx = net::fx_ready(&t.id);
    let (xf, xth, xqh) = layout_x(c, t, fx.as_deref(), w);
    let ko = layout_ko(c, t, prev, w, xth, xqh);
    let track_h = ko.h.max(xf.h);
    CardLay { tw: t.clone(), x, y, w, track_h, in_chain, root, ko, xf }
}

/// 면 그림 캐시 서명 — 바뀌면 다시 그린다(올린 곳 · 복사 표시 · 받은 그림 · 원문 정보).
pub fn face_sig(card: &CardLay, face: u8, hover: Option<&Act>, copied: Option<(&str, bool)>, pal: &Palette) -> String {
    let f = card.face(face);
    let hov = hover.filter(|a| f.hits.iter().any(|(_, h)| h == *a));
    let cp = copied.filter(|(id, _)| *id == card.tw.id);
    let ready: Vec<bool> = f.urls.iter().map(|u| net::image(u).is_some()).collect();
    let fx = f.kind == Kind::X && net::fx_ready(&card.tw.id).is_some();
    format!("{}|{:?}|{:?}|{:?}|{}|{}|{}|{fx}", pal.key(), hov, cp, ready, card.w, card.track_h, f.prims.len())
}

/// 면 하나를 그린다(카드 폭 × 칸 높이). 바탕 상자는 칸 높이 전체.
pub fn render_face(
    c: &Ctx,
    card: &CardLay,
    face: u8,
    hover: Option<&Act>,
    copied: Option<(&str, bool)>,
    painter: &mut TextPainter,
) -> Option<Pixmap> {
    let p = |v: f32| c.p(v);
    let f = card.face(face);
    let (w, h) = (card.w.ceil().max(1.0) as u32, card.track_h.ceil().max(1.0) as u32);
    let mut pm = Pixmap::new(w, h)?;
    let full = R::new(0.0, 0.0, card.w, card.track_h);
    match f.kind {
        Kind::Ko => view::boxed(&mut pm, full, p(12.0), Some(c.pal.paper()), Some((c.pal.mi_a(0.26), p(1.0)))),
        Kind::X => view::boxed(&mut pm, full, p(12.0), Some(xc::BG.color(1.0)), Some((xc::LINE.color(1.0), p(1.0)))),
    }
    let hot = |a: &Act| hover == Some(a);
    let mut txt: Vec<Txt> = Vec::new();
    // 글자는 도형 다음에 한꺼번에 — 단, 칸(그림) 위 글자(+N)는 그림 다음이어야 하므로 순서대로 모았다가 비운다.
    let flush = |pm: &mut Pixmap, txt: &mut Vec<Txt>, painter: &mut TextPainter| {
        painter.draw(pm, txt);
        txt.clear();
    };
    for pr in &f.prims {
        match pr {
            Prim::Box { r, rad, bg, line } => {
                flush(&mut pm, &mut txt, painter);
                view::boxed(&mut pm, *r, *rad, *bg, *line);
            }
            Prim::Hover { r, rad, act, color } => {
                if hot(act) {
                    flush(&mut pm, &mut txt, painter);
                    view::fill(&mut pm, &view::rrect(*r, *rad), *color, None);
                }
            }
            Prim::Circle { cx, cy, r, color } => view::fill(&mut pm, &view::circle(*cx, *cy, *r), *color, None),
            Prim::Avatar { src, cx, cy, r } => {
                let img: Option<Rc<Pixmap>> = match src {
                    AvSrc::Url(u, _) => net::image(u),
                    AvSrc::Member(_) => None,
                };
                let key = match src {
                    AvSrc::Member(k) | AvSrc::Url(_, k) => k,
                };
                match (&img, c.avatars.iter().find(|a| &a.key == key)) {
                    (Some(i), _) => view::draw_avatar(&mut pm, i, *cx, *cy, *r, 1.0),
                    (None, Some(a)) => view::draw_avatar(&mut pm, &a.img, *cx, *cy, *r, 1.0),
                    _ => view::fill(&mut pm, &view::circle(*cx, *cy, *r), Rgb::hex(0x111111).color(1.0), None),
                }
            }
            Prim::Cell { r, corners, url, video, gif, more, x, bg } => {
                flush(&mut pm, &mut txt, painter);
                let path = view::rrect4(*r, *corners);
                view::fill(&mut pm, &path, *bg, None);
                if let Some(img) = net::image(url) {
                    view::draw_cover(&mut pm, &img, *r, &path);
                }
                if *video {
                    draw_play(&mut pm, c, r, *x);
                }
                if *gif {
                    let b = R::new(r.x + p(8.0), r.y + r.h - p(28.0), p(36.0), p(20.0));
                    view::fill(&mut pm, &view::rrect(b, p(4.0)), BLACK.color(0.77), None);
                    txt.push(Txt { font: c.fonts.get(13.0, true), text: "GIF".into(), rect: b, color: WHITE.color(1.0), fmt: CENTER1, clip: None });
                }
                if *more > 0 {
                    view::fill(&mut pm, &path, BLACK.color(0.45), None);
                    txt.push(Txt { font: c.fonts.get(20.0, true), text: format!("+{more}"), rect: *r, color: WHITE.color(1.0), fmt: CENTER1, clip: None });
                }
            }
            Prim::Icon { kind, x, y, size, color, hover } => {
                let col = match hover {
                    Some((a, hc)) if hot(a) => *hc,
                    _ => *color,
                };
                draw_icon(&mut pm, *kind, *x, *y, *size, col);
            }
            Prim::Text { size, bold, text, r, color, fmt, hover } => {
                let col = match hover {
                    Some((a, hc)) if hot(a) => *hc,
                    _ => *color,
                };
                txt.push(Txt { font: c.fonts.get(*size, *bold), text: text.clone(), rect: *r, color: col, fmt: *fmt, clip: None });
            }
            Prim::CopyLabel { r, id, color } => {
                let label = match copied {
                    Some((cid, ok)) if cid == id => {
                        if ok { "복사됨" } else { "복사 실패" }
                    }
                    _ => "URL 복사",
                };
                txt.push(Txt { font: c.fonts.get(11.0, true), text: label.into(), rect: *r, color: *color, fmt: CENTER1, clip: None });
            }
        }
    }
    painter.draw(&mut pm, &txt);
    Some(pm)
}

/// ▶ — 번역 면: 멤버 색을 어둡게 + 흰 테(.tws-md__play 44px), 원문 면: X 파란 동그라미.
fn draw_play(pm: &mut Pixmap, c: &Ctx, r: &R, x: bool) {
    let p = |v: f32| c.p(v);
    let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    if x {
        view::fill(pm, &view::circle(cx, cy, p(26.0)), WHITE.color(1.0), None);
        view::fill(pm, &view::circle(cx, cy, p(23.0)), xc::BLUE.color(1.0), None);
    } else {
        view::fill(pm, &view::circle(cx, cy, p(22.0)), WHITE.color(1.0), None);
        view::fill(pm, &view::circle(cx, cy, p(20.0)), c.pal.m.mix(BLACK, 0.7).color(1.0), None);
    }
    // M4 2.2v9.6L11.8 7z (14 단위) · 18px · 오른쪽으로 3
    let size = if x { p(22.0) } else { p(18.0) };
    let u = size / 14.0;
    let (ox, oy) = (cx - size / 2.0 + p(if x { 2.0 } else { 1.5 }), cy - size / 2.0);
    let path = Pen::new(ox, oy, u).m(4.0, 2.2).l(4.0, 11.8).l(11.8, 7.0).z().path();
    view::fill(pm, &path, WHITE.color(1.0), None);
}

fn draw_icon(pm: &mut Pixmap, kind: Icon, x: f32, y: f32, size: f32, col: Color) {
    match kind {
        Icon::XLogo => view::draw_x_logo(pm, x, y, size, col),
        Icon::Reply => {
            // 0 0 12 12: M3 1.5v4.2a2 2 0 0 0 2 2h4.2 · M7.2 5.5l2 2.2-2 2.2
            let u = size / 12.0;
            let path = Pen::new(x, y, u)
                .m(3.0, 1.5)
                .l(3.0, 5.7)
                .q(3.0, 7.7, 5.0, 7.7)
                .l(9.2, 7.7)
                .m(7.2, 5.5)
                .l(9.2, 7.7)
                .l(7.2, 9.9)
                .path();
            view::stroke(pm, &path, col, 1.4 * u, None);
        }
        Icon::Heart => {
            let u = size / 24.0;
            let path = Pen::new(x, y, u)
                .m(12.0, 20.5)
                .c(5.0, 15.5, 2.0, 12.0, 2.0, 8.5)
                .c(2.0, 5.5, 4.3, 3.5, 7.0, 3.5)
                .c(9.0, 3.5, 11.0, 4.8, 12.0, 6.5)
                .c(13.0, 4.8, 15.0, 3.5, 17.0, 3.5)
                .c(19.7, 3.5, 22.0, 5.5, 22.0, 8.5)
                .c(22.0, 12.0, 19.0, 15.5, 12.0, 20.5)
                .z()
                .path();
            view::stroke(pm, &path, col, 1.8 * u, None);
        }
        Icon::Bubble => {
            let u = size / 24.0;
            let path = Pen::new(x, y, u)
                .m(7.0, 4.0)
                .l(17.0, 4.0)
                .q(21.0, 4.0, 21.0, 8.0)
                .l(21.0, 12.0)
                .q(21.0, 16.0, 17.0, 16.0)
                .l(12.0, 16.0)
                .l(7.0, 20.0)
                .l(7.0, 16.0)
                .q(3.0, 16.0, 3.0, 12.0)
                .l(3.0, 8.0)
                .q(3.0, 4.0, 7.0, 4.0)
                .z()
                .path();
            view::stroke(pm, &path, col, 1.8 * u, None);
        }
        Icon::Link => {
            let u = size / 24.0;
            let path = Pen::new(x, y, u)
                .m(9.5, 14.5)
                .l(14.5, 9.5)
                .m(11.0, 7.0)
                .l(13.0, 5.0)
                .q(16.0, 2.0, 19.0, 5.0)
                .q(22.0, 8.0, 19.0, 11.0)
                .l(17.0, 13.0)
                .m(13.0, 17.0)
                .l(11.0, 19.0)
                .q(8.0, 22.0, 5.0, 19.0)
                .q(2.0, 16.0, 5.0, 13.0)
                .l(7.0, 11.0)
                .path();
            view::stroke(pm, &path, col, 1.8 * u, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x_time_format() {
        // 2026-09-06 10:55Z = KST 19:55
        let ms = DateTime::parse_from_rfc3339("2026-09-06T10:55:00Z").unwrap().timestamp_millis();
        assert_eq!(x_time(ms), "오후 7:55 · 2026년 9월 6일");
        let ms = DateTime::parse_from_rfc3339("2026-09-06T15:05:00Z").unwrap().timestamp_millis();
        assert_eq!(x_time(ms), "오전 12:05 · 2026년 9월 7일");
    }

    #[test]
    fn compact_counts() {
        assert_eq!(compact(639), "639");
        assert_eq!(compact(12_345), "1.2만");
        assert_eq!(compact(20_000), "2만");
    }
}
