//! 시트 배치 · 합치기 — 머리 줄(멤버 칩 · 「번역 · 원문」 탭) · 방송 줄 · 피드(날짜 구분선 · 글타래 · 카드 · 점 줄).
//!
//! 카드는 card.rs 가 그린 면 그림을 붙이기만 한다. 밀거나 넘기는 카드는 두 면을 이동 · 투명도 · 배율로 겹친다
//! (원본 「겹쳐 밀기」: 앞 면은 손을 따라 밀리며 흐려지고, 뒤 면은 0.985 배에서 커지며 나타난다).

use std::collections::HashMap;

use tiny_skia::Pixmap;
use windows::Win32::Graphics::Gdi::{DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_SINGLELINE, DT_VCENTER};

use super::card::{self, Act, CardLay, Ctx};
use super::view::{self, BLACK, BUBBLE_W, Frame, R, Rgb, TextPainter, Txt};
use super::{PanelData, Unit};

const HEAD_H: f32 = 64.0;
const LIVE_ROW_H: f32 = 36.0;
const FEED_PAD_T: f32 = 12.0;
const FEED_PAD_X: f32 = 14.0;
const FEED_PAD_B: f32 = 22.0;
const GAP: f32 = 12.0;
const EMPTY_H: f32 = 120.0;
/// 미니 예고 카드 띠(위 6 + 카드 60 + 아래 10)
const FOOT_H: f32 = 76.0;
const MINI_H: f32 = 60.0;

const LEFT1: DRAW_TEXT_FORMAT = DRAW_TEXT_FORMAT(DT_LEFT.0 | DT_VCENTER.0 | DT_SINGLELINE.0);
const CENTER1: DRAW_TEXT_FORMAT = DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0);
const CENTER1E: DRAW_TEXT_FORMAT = DRAW_TEXT_FORMAT(DT_CENTER.0 | DT_VCENTER.0 | DT_SINGLELINE.0 | DT_END_ELLIPSIS.0);

pub struct ChipLay {
    pub key: String,
    pub r: R,
    pub current: bool,
    pub enabled: bool,
}

pub enum Block {
    Day { r: R, label: String },
    Chain { r: R },
    Card(Box<CardLay>),
}

pub struct Layout {
    pub s: f32,
    pub chips: Vec<ChipLay>,
    /// 탭 상자 · 단추 두 개
    pub seg: (R, [R; 2]),
    pub live: Option<(R, String)>,
    pub feed_top: f32,
    pub blocks: Vec<Block>,
    pub content_h: f32,
    pub empty: bool,
    /// 아래 미니 예고 카드 띠 높이(없으면 0)
    pub foot_h: f32,
}

impl Layout {
    pub fn build(data: &PanelData, c: &Ctx) -> Layout {
        let s = c.s;
        let p = |v: f32| v * s;
        let bw = p(BUBBLE_W);
        let me = c.me;
        let fonts = c.fonts;

        // 「번역 · 원문」 탭(오른쪽): 상자 = 테두리 1 + 안쪽 2, 단추 = 글자 + 옆 9 · 높이 28
        let lw = |t: &str| me.width(fonts.get(12.0, true), t) + p(18.0);
        let (b0, b1) = (lw("번역"), lw("원문"));
        let sw = b0 + b1 + p(6.0);
        let sr = R::new(bw - p(12.0) - sw, p(19.0), sw, p(34.0));
        let seg = (sr, [R::new(sr.x + p(3.0), sr.y + p(3.0), b0, p(28.0)), R::new(sr.x + p(3.0) + b0, sr.y + p(3.0), b1, p(28.0))]);

        // 멤버 칩: 지금 멤버 = 얼굴 + 이름 알약, 다른 멤버 = 얼굴만
        let mut chips = Vec::new();
        let mut x = p(12.0);
        for ch in &data.chips {
            let current = ch.key == data.member;
            let w = if current { p(2.0 + 32.0 + 8.0 + 14.0) + me.width(fonts.get(14.0, true), &data.name_ko) } else { p(36.0) };
            chips.push(ChipLay { key: ch.key.clone(), r: R::new(x, p(18.0), w, p(36.0)), current, enabled: ch.enabled });
            x += w + p(8.0);
        }

        let mut y = p(HEAD_H);
        let live = data.live.as_ref().map(|(title, _)| {
            let r = R::new(p(FEED_PAD_X), y, bw - p(2.0 * FEED_PAD_X), p(30.0));
            y += p(LIVE_ROW_H);
            (r, format!("▶  방송 보러 가기 · {title}"))
        });
        let feed_top = y;

        let inner_x = p(FEED_PAD_X);
        let inner_w = bw - p(2.0 * FEED_PAD_X);
        let mut blocks = Vec::new();
        let mut cy = p(FEED_PAD_T);
        let mut prev_day = String::new();
        let mut first = true;
        for Unit { day, tweets } in &data.units {
            if !day.is_empty() && *day != prev_day {
                if !first {
                    cy += p(GAP);
                }
                first = false;
                let tw = me.width(fonts.get(11.5, true), day) + p(20.0);
                let h = p(11.5 * 1.55 + 4.0).round();
                blocks.push(Block::Day { r: R::new(inner_x + (inner_w - tw) / 2.0, cy, tw, h), label: day.clone() });
                cy += h;
                prev_day = day.clone();
            }
            if !first {
                cy += p(GAP);
            }
            first = false;
            let chain = tweets.len() > 1;
            let (cx, cw) = if chain { (inner_x + p(18.0), inner_w - p(18.0)) } else { (inner_x, inner_w) };
            let top = cy;
            for (i, t) in tweets.iter().enumerate() {
                if i > 0 {
                    cy += p(GAP);
                }
                let card = card::layout_card(c, t, if i > 0 { tweets.get(i - 1) } else { None }, cx, cy, cw, chain, chain && i == 0);
                cy += card.h(s);
                blocks.push(Block::Card(Box::new(card)));
            }
            if chain {
                blocks.push(Block::Chain { r: R::new(inner_x, top, inner_w, cy - top) });
            }
        }
        let empty = data.units.is_empty();
        let content_h = if empty { p(EMPTY_H) } else { cy + p(FEED_PAD_B) };
        let foot_h = if data.upcoming.is_some() { p(FOOT_H) } else { 0.0 };
        Layout { s, chips, seg, live, feed_top, blocks, content_h, empty, foot_h }
    }

    pub fn cards(&self) -> impl Iterator<Item = &CardLay> {
        self.blocks.iter().filter_map(|b| if let Block::Card(c) = b { Some(c.as_ref()) } else { None })
    }

    pub fn card(&self, id: &str) -> Option<&CardLay> {
        self.cards().find(|c| c.tw.id == id)
    }
}

pub fn max_scroll(lay: &Layout, fr: &Frame) -> f32 {
    (lay.content_h - (fr.bh - lay.feed_top - lay.foot_h)).max(0.0)
}

/// 피드 칸(창 좌표).
pub fn feed_rect(lay: &Layout, fr: &Frame) -> R {
    R::new(fr.bx, fr.by + lay.feed_top, BUBBLE_W * lay.s, fr.bh - lay.feed_top - lay.foot_h)
}

/// 화면에 보이는 카드(창 좌표 y 로).
pub fn visible_cards<'a>(lay: &'a Layout, fr: &Frame, scroll: f32) -> Vec<&'a CardLay> {
    let feed = feed_rect(lay, fr);
    let fy = fr.by + lay.feed_top - scroll;
    lay.cards()
        .filter(|c| c.y + fy < feed.y + feed.h && c.y + c.h(lay.s) + fy > feed.y)
        .collect()
}

/// 면 하나의 보이는 모양.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vis {
    /// 가로 이동(px)
    pub tx: f32,
    pub op: f32,
    pub sc: f32,
}

impl Vis {
    pub const FRONT: Vis = Vis { tx: 0.0, op: 1.0, sc: 1.0 };
    pub const BACK: Vis = Vis { tx: 0.0, op: 0.0, sc: 0.985 };
}

/// 카드 하나의 지금 모양.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CardVis {
    /// 앞(맨 위) 면
    pub front: u8,
    /// [번역, 원문] 면 모양
    pub vis: [Vis; 2],
    /// 점 줄 — 원문 쪽으로 간 정도(0~1)
    pub dots: f32,
    /// 움직이는 중(두 면을 겹쳐 그린다)
    pub active: bool,
}

impl CardVis {
    pub fn rest(face: u8) -> CardVis {
        let mut vis = [Vis::BACK; 2];
        vis[face as usize] = Vis::FRONT;
        CardVis { front: face, vis, dots: face as f32, active: false }
    }
}

pub struct ViewState<'a> {
    pub scroll: f32,
    pub hover: Option<&'a Act>,
    /// 탭 알약 자리(0 = 번역, 1 = 원문)
    pub seg: f32,
    /// 카드 기본 면
    pub face: u8,
    pub cards: &'a HashMap<String, CardVis>,
    pub faces: &'a HashMap<(String, u8), (String, Pixmap)>,
    /// 지금(UTC ms) — 도넛 타이머
    pub now_ms: i64,
}

pub struct Rendered {
    pub pm: Pixmap,
    pub hits: Vec<(R, Act)>,
}

pub fn render(data: &PanelData, lay: &Layout, fr: &Frame, st: &ViewState, c: &Ctx, bg: &Pixmap, painter: &mut TextPainter) -> Option<Rendered> {
    let s = lay.s;
    let p = |v: f32| v * s;
    if bg.width() != fr.w as u32 || bg.height() != fr.h as u32 {
        return None;
    }
    let pal = c.pal;
    let mut pm = bg.clone();
    let mut hits: Vec<(R, Act)> = Vec::new();
    let mut txt: Vec<Txt> = Vec::new();
    let (ox, oy) = (fr.bx, fr.by);
    let bw = p(BUBBLE_W);
    let sub = pal.sub();
    let hot = |a: &Act| st.hover == Some(a);
    let av = |k: &str| c.avatars.iter().find(|a| a.key == k);

    // ── 머리 줄: 멤버 칩 ──
    for ch in &lay.chips {
        let r = ch.r.off(ox, oy);
        let a = av(&ch.key);
        let lane = a.and_then(|a| a.color).unwrap_or(Rgb(136.0, 136.0, 136.0));
        let (acx, acy) = (r.x + p(18.0), r.y + r.h / 2.0);
        if ch.current {
            view::fill(&mut pm, &view::rrect(r, r.h / 2.0), pal.mi.color(1.0), None);
            txt.push(Txt {
                font: c.fonts.get(14.0, true),
                text: data.name_ko.clone(),
                rect: R::new(r.x + p(42.0), r.y, r.w - p(42.0), r.h),
                color: pal.m.color(1.0),
                fmt: LEFT1,
                clip: None,
            });
        } else if ch.enabled {
            hits.push((r, Act::Chip(ch.key.clone())));
        }
        let op = if ch.enabled || ch.current { 1.0 } else { 0.4 };
        view::fill(&mut pm, &view::circle(acx, acy, p(17.0)), BLACK.color(0.35 * op), None);
        view::fill(&mut pm, &view::circle(acx, acy, p(16.0)), lane.color(op), None);
        if let Some(a) = a {
            view::draw_avatar(&mut pm, if ch.enabled || ch.current { &a.img } else { &a.gray }, acx, acy, p(14.5), op);
        }
        if !ch.current && hot(&Act::Chip(ch.key.clone())) {
            view::stroke(&mut pm, &view::circle(acx, acy, p(17.0)), pal.mi.color(1.0), p(2.0), None);
        }
    }

    // ── 「번역 · 원문」 탭 ──
    let (sr, sb) = lay.seg;
    let sr = sr.off(ox, oy);
    view::boxed(&mut pm, sr, p(9.0), Some(pal.mi_a(0.14)), Some((pal.mi_a(0.26), p(1.0))));
    let (b0, b1) = (sb[0].off(ox, oy), sb[1].off(ox, oy));
    let t = st.seg.clamp(0.0, 1.0);
    let pill = R::new(b0.x + (b1.x - b0.x) * t, b0.y, b0.w + (b1.w - b0.w) * t, b0.h);
    view::fill(&mut pm, &view::rrect(pill, p(7.0)), pal.mi.color(1.0), None);
    for (i, (r, label)) in [(b0, "번역"), (b1, "원문")].into_iter().enumerate() {
        let on = (st.seg >= 0.5) == (i == 1);
        txt.push(Txt {
            font: c.fonts.get(12.0, on),
            text: label.into(),
            rect: r,
            color: if on { pal.m.color(1.0) } else { sub.color(1.0) },
            fmt: CENTER1,
            clip: None,
        });
        hits.push((r, Act::Seg(i as u8)));
    }

    // ── 방송 보러 가기 ──
    if let (Some((r, label)), Some(_)) = (&lay.live, &data.live) {
        let r = r.off(ox, oy);
        view::fill(&mut pm, &view::rrect(r, r.h / 2.0), pal.mi.color(if hot(&Act::Live) { 1.0 } else { 0.92 }), None);
        txt.push(Txt {
            font: c.fonts.get(12.5, true),
            text: label.clone(),
            rect: R::new(r.x + p(14.0), r.y, r.w - p(28.0), r.h),
            color: pal.m.color(1.0),
            fmt: CENTER1E,
            clip: None,
        });
        hits.push((r, Act::Live));
    }
    painter.draw(&mut pm, &txt);
    txt.clear();

    // ── 피드 ──
    let feed = feed_rect(lay, fr);
    let fy = oy + lay.feed_top - st.scroll;
    let feed_mask = view::clip_mask(fr.w as u32, fr.h as u32, feed, None);
    let visible = |r: &R| r.y + fy < feed.y + feed.h && r.y + r.h + fy > feed.y;

    if lay.empty {
        txt.push(Txt {
            font: c.fonts.get(14.0, false),
            text: format!("{}의 트윗이 아직 없어요", data.name_ko),
            rect: feed,
            color: sub.color(1.0),
            fmt: CENTER1,
            clip: Some(feed),
        });
    }

    // 글타래 세로줄(카드 아래)
    for b in &lay.blocks {
        if let Block::Chain { r } = b {
            if visible(r) {
                let r = r.off(ox, fy);
                let line = R::new(r.x + p(6.0), r.y + p(16.0), p(2.0), (r.h - p(46.0)).max(0.0));
                view::fill(&mut pm, &view::rrect(line, p(1.0)), pal.mi_a(0.5), feed_mask.as_ref());
            }
        }
    }
    for b in &lay.blocks {
        match b {
            Block::Day { r, label } if visible(r) => {
                let r = r.off(ox, fy);
                view::fill(&mut pm, &view::rrect(r, r.h / 2.0), pal.mi_a(0.14), feed_mask.as_ref());
                txt.push(Txt { font: c.fonts.get(11.5, true), text: label.clone(), rect: r, color: sub.color(1.0), fmt: CENTER1, clip: Some(feed) });
            }
            Block::Card(card) => {
                let ch = card.h(s);
                if !(card.y + fy < feed.y + feed.h && card.y + ch + fy > feed.y) {
                    continue;
                }
                draw_card(&mut pm, card, ox, fy, st, c, &feed, feed_mask.as_ref(), &mut hits);
            }
            _ => {}
        }
    }
    painter.draw(&mut pm, &txt);

    // ── 끝 흐림 · 스크롤 막대 · ↓ 동그라미 ──
    let maxs = max_scroll(lay, fr);
    let fade_t = if st.scroll > 0.5 { p(12.0) } else { 0.0 };
    let fade_b = if st.scroll < maxs - 0.5 { p(16.0) } else { 0.0 };
    view::fade_rows(&mut pm, bg, feed, fade_t, fade_b);
    if maxs > 0.5 {
        let track_h = feed.h - p(8.0);
        let thumb_h = (track_h * feed.h / (feed.h + maxs)).max(p(28.0));
        let ty = feed.y + p(4.0) + (track_h - thumb_h) * (st.scroll / maxs);
        view::fill(&mut pm, &view::rrect(R::new(ox + bw - p(8.0), ty, p(4.0), thumb_h), p(2.0)), pal.mi_a(0.26), None);
    }
    if maxs > 0.5 && st.scroll < maxs - p(40.0) {
        let d = p(44.0);
        let r = R::new(ox + bw - p(16.0) - d, feed.y + feed.h - p(16.0) - d, d, d);
        let (cx, cy) = (r.x + d / 2.0, r.y + d / 2.0);
        for i in 0..6 {
            view::fill(&mut pm, &view::circle(cx, cy + p(4.0), d / 2.0 - p(4.0) + i as f32 * p(1.6)), BLACK.color(0.05), None);
        }
        let sc = if hot(&Act::Fab) { 1.07 } else { 1.0 };
        view::fill(&mut pm, &view::circle(cx, cy, d / 2.0 * sc), pal.mi.color(1.0), None);
        let path = view::Pen::new(cx - p(10.0), cy - p(10.0), p(1.0)).m(10.0, 4.0).l(10.0, 15.0).m(5.0, 10.5).l(10.0, 15.5).l(15.0, 10.5).path();
        view::stroke(&mut pm, &path, pal.m.color(1.0), p(2.2), None);
        hits.push((r, Act::Fab));
    }

    // ── 미니 예고 카드(24시간 안 방송) ──
    if let Some(up) = &data.upcoming {
        let r = R::new(ox + p(FEED_PAD_X), oy + fr.bh - lay.foot_h + p(6.0), bw - p(2.0 * FEED_PAD_X), p(MINI_H));
        let lane = c.avatars.iter().find(|a| a.key == data.member).and_then(|a| a.color).unwrap_or(Rgb::hex(0x7c8aa0));
        draw_mini(&mut pm, painter, c, up, r, lane, st.now_ms, hot(&Act::Open(up.url.clone())));
        hits.push((r, Act::Open(up.url.clone())));
    }

    // 피드 밖으로 나간 카드 단추는 누를 수 없게
    hits.retain(|(r, a)| match a {
        Act::Copy(_) | Act::Open(_) | Act::Video { .. } | Act::Track(_) => {
            r.y + r.h / 2.0 >= feed.y && r.y + r.h / 2.0 <= feed.y + feed.h
        }
        _ => true,
    });
    Some(Rendered { pm, hits })
}

#[allow(clippy::too_many_arguments)]
fn draw_card(
    pm: &mut Pixmap,
    card: &CardLay,
    ox: f32,
    fy: f32,
    st: &ViewState,
    c: &Ctx,
    feed: &R,
    feed_mask: Option<&tiny_skia::Mask>,
    hits: &mut Vec<(R, Act)>,
) {
    let s = c.s;
    let p = |v: f32| v * s;
    let pal = c.pal;
    let (x, y) = ((card.x + ox).round(), (card.y + fy).round());
    let cv = st.cards.get(&card.tw.id).copied().unwrap_or_else(|| CardVis::rest(st.face));

    // 글타래 점: 왼쪽 -15 · 위 18, 8px, 바깥 3px 멤버 색 고리. 뿌리 = 속 빈 점
    if card.in_chain {
        let (dx, dy) = (x - p(11.0), y + p(22.0));
        view::fill(pm, &view::circle(dx, dy, p(7.0)), pal.m.color(1.0), feed_mask);
        view::fill(pm, &view::circle(dx, dy, p(4.0)), pal.mi.color(1.0), feed_mask);
        if card.root {
            view::fill(pm, &view::circle(dx, dy, p(2.0)), pal.m.color(1.0), feed_mask);
        }
    }

    let track = R::new(x, y, card.w, card.track_h);
    hits.push((R::new(x, y, card.w, card.h(s)), Act::Track(card.tw.id.clone())));
    let img = |f: u8| st.faces.get(&(card.tw.id.clone(), f)).map(|(_, pm)| pm);
    if !cv.active {
        if let Some(face) = img(cv.front) {
            view::blit(pm, face, x, y, 1.0, 1.0, feed_mask);
        }
        for (r, a) in &card.face(cv.front).hits {
            hits.push((r.off(x, y), a.clone()));
        }
    } else {
        // 칸(모서리 14) 밖은 자른다(overflow: hidden)
        let clip = feed.intersect(&track).and_then(|_| view::clip_mask(pm.width(), pm.height(), *feed, Some((track, p(14.0)))));
        let back = 1 - cv.front;
        for f in [back, cv.front] {
            let v = cv.vis[f as usize];
            if let Some(face) = img(f) {
                view::blit(pm, face, x + v.tx, y, v.sc, v.op, clip.as_ref());
            }
        }
    }

    // 면 점 2개(끌기 손잡이 겸 — 끄는 동안 폭이 따라간다)
    let d = cv.dots.clamp(0.0, 1.0);
    let (w0, w1) = (p(6.0 + 8.0 * (1.0 - d)), p(6.0 + 8.0 * d));
    let gap = p(5.0);
    let dy = y + card.track_h + p(4.0 + 10.0);
    let mut dx = x + card.w / 2.0 - (w0 + w1 + gap) / 2.0;
    for (wd, on) in [(w0, 1.0 - d), (w1, d)] {
        let r = R::new(dx, dy - p(3.0), wd, p(6.0));
        view::fill(pm, &view::rrect(r, p(3.0)), pal.mi_a(0.35 + 0.55 * on), feed_mask);
        dx += wd + gap;
    }
}

/// 두 면 중 하나가 화면 근처에서 필요한가(그림을 미리 그려 둘 면).
pub fn needed_faces(cv: &CardVis) -> Vec<u8> {
    if cv.active { vec![0, 1] } else { vec![cv.front] }
}

/// 미니 예고 카드(.tcard 를 한 줄 띠로): 썸네일 16:9 · 날짜 시각 + 태그 · 제목 · 작은 도넛 타이머.
#[allow(clippy::too_many_arguments)]
fn draw_mini(pm: &mut Pixmap, painter: &mut TextPainter, c: &Ctx, up: &super::Upcoming, r: R, lane: Rgb, now_ms: i64, hot: bool) {
    let p = |v: f32| v * c.s;
    // 카드 바탕 = 예고판 카드(--color-bg #101015 · 테두리 · 모서리 12)
    let card_bg = Rgb::hex(0x101015);
    view::boxed(pm, r, p(12.0), Some(card_bg.color(1.0)), Some((view::WHITE.color(if hot { 0.28 } else { 0.12 }), p(1.0))));
    let pad = p(6.0);
    let th = R::new(r.x + pad, r.y + pad, (r.h - 2.0 * pad) * 16.0 / 9.0, r.h - 2.0 * pad);
    let path = view::rrect(th, p(8.0));
    view::fill(pm, &path, lane.mix(BLACK, 0.45).color(1.0), None);
    if let Some(img) = up.thumb.as_deref().and_then(crate::net::image) {
        view::draw_cover(pm, &img, th, &path);
    }
    let mut txt = Vec::new();
    if up.membership {
        txt.push(Txt { font: c.fonts.get(16.0, false), text: "🔒".into(), rect: th, color: view::WHITE.color(1.0), fmt: CENTER1, clip: None });
    }

    // 도넛(40px, 두께 4) — 오른쪽
    let d = p(40.0);
    let (cx, cy) = (r.x + r.w - p(10.0) - d / 2.0, r.y + r.h / 2.0);
    let late_c = Rgb::hex(0xe0a030);
    let (ring, big, small) = super::ring_of(up.start_ms - now_ms);
    view::fill(pm, &view::circle(cx, cy, d / 2.0), view::WHITE.color(0.1), None);
    match ring {
        super::Ring::Day(f) => view::fill(pm, &wedge(cx, cy, d / 2.0, 0.0, f), lane.color(1.0), None),
        // 12시부터 시계방향으로 (1 − 남은 비율)만큼 빈칸, 나머지는 노랑
        super::Ring::Min(f) => view::fill(pm, &wedge(cx, cy, d / 2.0, 1.0 - f, 1.0), late_c.color(1.0), None),
        super::Ring::Late => {}
    }
    view::fill(pm, &view::circle(cx, cy, d / 2.0 - p(4.0)), card_bg.color(1.0), None);
    let text_c = Rgb::hex(0xe6e6e6);
    let muted = Rgb::hex(0x999999);
    let big_c = if ring == super::Ring::Late { late_c } else { text_c };
    // 링 안(지름 - 두께 양쪽 - 여백)에 넘치면 한 단계 작게
    let big_size = if c.me.width(c.fonts.get(11.0, true), &big) > d - p(12.0) { 9.5 } else { 11.0 };
    if small.is_empty() {
        txt.push(Txt { font: c.fonts.get(big_size, true), text: big, rect: R::new(cx - d / 2.0, cy - p(8.0), d, p(16.0)), color: big_c.color(1.0), fmt: CENTER1, clip: None });
    } else {
        txt.push(Txt { font: c.fonts.get(big_size, true), text: big, rect: R::new(cx - d / 2.0, cy - p(11.0), d, p(13.0)), color: big_c.color(1.0), fmt: CENTER1, clip: None });
        txt.push(Txt { font: c.fonts.get(7.5, false), text: small, rect: R::new(cx - d / 2.0, cy + p(1.0), d, p(10.0)), color: muted.color(1.0), fmt: CENTER1, clip: None });
    }

    // 가운데: 1줄 = 날짜 시각(굵게) + 합동 · 회원 전용 태그, 2줄 = 제목(한 줄)
    let tx = th.x + th.w + p(10.0);
    let tw = cx - d / 2.0 - p(10.0) - tx;
    let when = super::when_label(up.start_ms);
    let ww = c.me.width(c.fonts.get(12.5, true), &when);
    txt.push(Txt { font: c.fonts.get(12.5, true), text: when, rect: R::new(tx, r.y + p(9.0), ww + p(2.0), p(18.0)), color: text_c.color(1.0), fmt: LEFT1, clip: None });
    let mut tagx = tx + ww + p(8.0);
    for (on, label) in [(up.collab, "합동"), (up.membership, "회원 전용")] {
        if !on {
            continue;
        }
        let lw = c.me.width(c.fonts.get(10.5, true), label) + p(12.0);
        if tagx + lw > tx + tw {
            break;
        }
        let tr = R::new(tagx, r.y + p(10.0), lw, p(16.0));
        view::fill(pm, &view::rrect(tr, p(8.0)), lane.color(0.28), None);
        txt.push(Txt { font: c.fonts.get(10.5, true), text: label.into(), rect: tr, color: text_c.color(1.0), fmt: CENTER1, clip: None });
        tagx += lw + p(4.0);
    }
    txt.push(Txt {
        font: c.fonts.get(13.5, true),
        text: up.title.clone(),
        rect: R::new(tx, r.y + p(30.0), tw.max(0.0), p(20.0)),
        color: text_c.color(1.0),
        fmt: DRAW_TEXT_FORMAT(LEFT1.0 | DT_END_ELLIPSIS.0),
        clip: None,
    });
    painter.draw(pm, &txt);
}

/// 12시부터 시계방향 a → b(0~1 바퀴) 부채꼴.
fn wedge(cx: f32, cy: f32, r: f32, a: f32, b: f32) -> Option<tiny_skia::Path> {
    if b - a <= 0.001 {
        return None;
    }
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(cx, cy);
    let n = ((b - a) * 72.0).ceil().max(2.0) as usize;
    for i in 0..=n {
        let t = a + (b - a) * i as f32 / n as f32;
        let ang = t * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
        pb.line_to(cx + r * ang.cos(), cy + r * ang.sin());
    }
    pb.close();
    pb.finish()
}
