//! 캐릭터 그림 — 유노 리그 조립 · 정지 그림 · 겹침 표시(LIVE · 새 트윗). Win32 없음.
//! 리그 원본 = mewtype-scheduler `src/frontend/js/bdayscene-yuno.js` 137~182행(drawRot · front).
//! 리그 좌표계 = 가로 240 · 세로 234, 바닥 = y 234(창의 캐릭터 영역 아랫변에 붙는다).

use tiny_skia::{
    Color, FillRule, FilterQuality, Mask, Paint, PathBuilder, Pattern, Pixmap, Rect, SpreadMode,
    Stroke, Transform,
};

use super::motion::Pose;

const RIG_H: f32 = 234.0;

/// 그릴 칸: 창 전체 크기(px)와 캐릭터 영역 높이(px). 캐릭터 영역은 창 아래쪽에 붙는다.
#[derive(Clone, Copy, Debug)]
pub struct Canvas {
    pub w: f32,
    pub h: f32,
    pub char_h: f32,
}

/// 2D 아핀 변환. x' = a·x + c·y + e, y' = b·x + d·y + f.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aff {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Aff {
    pub const ID: Aff = Aff { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn t(x: f32, y: f32) -> Aff {
        Aff { e: x, f: y, ..Aff::ID }
    }

    pub fn s(sx: f32, sy: f32) -> Aff {
        Aff { a: sx, d: sy, ..Aff::ID }
    }

    /// 원점 기준 회전(도).
    pub fn rot0(deg: f32) -> Aff {
        let r = deg.to_radians();
        let (s, c) = (r.sin(), r.cos());
        Aff { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 }
    }

    /// (ox, oy) 를 중심으로 deg 회전.
    pub fn rot(deg: f32, ox: f32, oy: f32) -> Aff {
        Aff::t(ox, oy).mul(Aff::rot0(deg)).mul(Aff::t(-ox, -oy))
    }

    /// self ∘ o — o 를 먼저 적용하고 self 를 적용한다.
    pub fn mul(self, o: Aff) -> Aff {
        Aff {
            a: self.a * o.a + self.c * o.b,
            b: self.b * o.a + self.d * o.b,
            c: self.a * o.c + self.c * o.d,
            d: self.b * o.c + self.d * o.d,
            e: self.a * o.e + self.c * o.f + self.e,
            f: self.b * o.e + self.d * o.f + self.f,
        }
    }

    pub fn apply(self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    fn to_skia(self) -> Transform {
        Transform::from_row(self.a, self.b, self.c, self.d, self.e, self.f)
    }
}

/// 리그 좌표 → 창 픽셀. flip = -1 이면 좌우 반전(리그 x 중심 120 기준).
/// 리그 바닥(y = 234, bounce 0)이 창 아랫변(h)에 온다.
fn rig_to_screen(cv: &Canvas, flip: f32, bounce: f32) -> Aff {
    let s = cv.char_h / RIG_H;
    Aff {
        a: flip * s,
        b: 0.0,
        c: 0.0,
        d: s,
        e: cv.w / 2.0 - flip * 120.0 * s,
        f: cv.h + (bounce - RIG_H) * s,
    }
}

/// 유노 부위 그림 10장 중 쓰는 9장(head_flat 은 소리 없을 때 눈 감은 얼굴용이라 쓰지 않는다).
pub struct YunoSprites {
    arm_l: Pixmap,
    arm_r: Pixmap,
    body: Pixmap,
    ear_l: Pixmap,
    ear_r: Pixmap,
    head_closed: Pixmap,
    head_open: Pixmap,
    headset: Pixmap,
    tail: Pixmap,
}

impl YunoSprites {
    pub fn load() -> Result<Self, String> {
        Ok(YunoSprites {
            arm_l: decode(include_bytes!("../../assets/yuno/arm_l.webp"))?,
            arm_r: decode(include_bytes!("../../assets/yuno/arm_r.webp"))?,
            body: decode(include_bytes!("../../assets/yuno/body.webp"))?,
            ear_l: decode(include_bytes!("../../assets/yuno/ear_l.webp"))?,
            ear_r: decode(include_bytes!("../../assets/yuno/ear_r.webp"))?,
            head_closed: decode(include_bytes!("../../assets/yuno/head_closed.webp"))?,
            head_open: decode(include_bytes!("../../assets/yuno/head_open.webp"))?,
            headset: decode(include_bytes!("../../assets/yuno/headset.webp"))?,
            tail: decode(include_bytes!("../../assets/yuno/tail.webp"))?,
        })
    }
}

/// 정지 멤버 그림(240×280, 임시본). 키가 없으면 None.
pub fn still_bytes(key: &str) -> Option<&'static [u8]> {
    match key {
        "arale" => Some(include_bytes!("../../assets/static/arale.png")),
        "nonoka" => Some(include_bytes!("../../assets/static/nonoka.png")),
        "ritsu" => Some(include_bytes!("../../assets/static/ritsu.png")),
        "miyako" => Some(include_bytes!("../../assets/static/miyako.png")),
        _ => None,
    }
}

pub fn live_badge_bytes() -> &'static [u8] {
    include_bytes!("../../assets/live_badge.png")
}

/// PNG/WebP 바이트 → 미리 곱한(premultiplied) RGBA Pixmap.
pub fn decode(bytes: &[u8]) -> Result<Pixmap, String> {
    let img = image::load_from_memory(bytes)
        .map_err(|e| format!("그림 해독 실패: {e}"))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let mut pm = Pixmap::new(w, h).ok_or("그림 크기가 잘못됨")?;
    for (src, dst) in img.as_raw().chunks_exact(4).zip(pm.data_mut().chunks_exact_mut(4)) {
        let a = src[3] as u32;
        for k in 0..3 {
            dst[k] = ((src[k] as u32 * a + 127) / 255) as u8;
        }
        dst[3] = src[3];
    }
    Ok(pm)
}

/// 그림 하나를 변환 t 로 그린다(t 는 그림 픽셀 좌표 → 캔버스 좌표).
fn draw_img(pm: &mut Pixmap, img: &Pixmap, t: Aff, mask: Option<&Mask>) {
    let Some(rect) = Rect::from_xywh(0.0, 0.0, img.width() as f32, img.height() as f32) else {
        return;
    };
    let paint = Paint {
        shader: Pattern::new(
            img.as_ref(),
            SpreadMode::Pad,
            FilterQuality::Bilinear,
            1.0,
            Transform::identity(),
        ),
        anti_alias: true,
        ..Paint::default()
    };
    pm.fill_rect(rect, &paint, t.to_skia(), mask);
}

/// 리그 부위 하나. 원본 drawRot(img, x, y, w, h, deg, ox, oy) 와 같다:
/// (ox, oy) 중심으로 deg 회전한 뒤 (x, y, w, h) 칸에 그린다.
#[allow(clippy::too_many_arguments)]
fn draw_part(
    pm: &mut Pixmap,
    img: &Pixmap,
    base: Aff,
    (x, y, w, h): (f32, f32, f32, f32),
    deg: f32,
    (ox, oy): (f32, f32),
    mask: Option<&Mask>,
) {
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let t = base
        .mul(Aff::rot(deg, ox, oy))
        .mul(Aff::t(x, y))
        .mul(Aff::s(w / iw, h / ih));
    draw_img(pm, img, t, mask);
}

/// 몸통 클립(웹 원본 다각형). gb = 몸통이 놓인 좌표계(회전 포함)에서 만든 마스크.
fn body_mask(cv: &Canvas, gb: Aff) -> Option<Mask> {
    const PTS: [(f32, f32); 10] = [
        (0.25, 0.0),
        (0.75, 0.0),
        (0.75, 0.28),
        (0.78, 0.66),
        (1.0, 0.85),
        (1.0, 1.0),
        (0.0, 1.0),
        (0.0, 0.85),
        (0.22, 0.66),
        (0.25, 0.28),
    ];
    let mut pb = PathBuilder::new();
    for (i, (u, v)) in PTS.iter().enumerate() {
        let (x, y) = gb.apply(59.0 + u * 126.0, 154.0 + v * 80.0);
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.close();
    let path = pb.finish()?;
    let mut m = Mask::new(cv.w as u32, cv.h as u32)?;
    m.fill_path(&path, FillRule::Winding, true, Transform::identity());
    Some(m)
}

/// 유노 한 프레임. 원본 front() 의 순서: 꼬리 → 몸 → 팔 → 머리. 캔버스를 먼저 지운다.
/// eyes_closed = 깜빡임 중 감은 눈 그림. flip = -1 이면 좌우 반전.
pub fn yuno_frame(
    pm: &mut Pixmap,
    sp: &YunoSprites,
    p: &Pose,
    eyes_closed: bool,
    cv: &Canvas,
    flip: f32,
) {
    pm.fill(Color::TRANSPARENT);
    let g = rig_to_screen(cv, flip, p.bounce as f32);
    let (tail, body_roll) = (p.tail as f32, p.body_roll as f32);
    let (left, right) = (p.left as f32, p.right as f32);
    let (ear, mic) = (p.ear as f32, p.mic as f32);
    let (head_y, head_angle) = (p.head_y as f32, p.head_angle as f32);

    // 1. 꼬리
    draw_part(
        pm,
        &sp.tail,
        g,
        (157.0, 159.0, 82.0, 70.0),
        tail,
        (157.0 + 0.15 * 82.0, 159.0 + 0.75 * 70.0),
        None,
    );

    // 2. 몸통: (120,170) 중심 회전 → 그 좌표계에서 다각형 클립 → 그림
    let gb = g.mul(Aff::rot(body_roll, 120.0, 170.0));
    let mask = body_mask(cv, gb);
    draw_part(pm, &sp.body, gb, (59.0, 154.0, 126.0, 80.0), 0.0, (0.0, 0.0), mask.as_ref());

    // 3. 팔
    draw_part(pm, &sp.arm_l, g, (50.0, 157.0, 43.0, 53.0), left, (86.0, 164.0), None);
    draw_part(pm, &sp.arm_r, g, (114.0, 157.0, 45.0, 52.0), right, (153.0, 164.0), None);

    // 4. 머리 묶음: 원점 (32,10), 회전 중심 (32+87.5, 10+0.88·156). 원본과 같은 순서로 곱한다.
    let (cx, cy) = (32.0 + 87.5, 10.0 + 0.88 * 156.0);
    let gh = g
        .mul(Aff::t(cx, cy))
        .mul(Aff::t(0.0, head_y))
        .mul(Aff::rot0(head_angle))
        .mul(Aff::t(-cx, -cy))
        .mul(Aff::t(32.0, 10.0));
    let head = if eyes_closed { &sp.head_closed } else { &sp.head_open };
    draw_part(pm, head, gh, (0.0, 0.0, 175.0, 156.0), 0.0, (0.0, 0.0), None);
    draw_part(pm, &sp.ear_l, gh, (0.0, 0.0, 175.0, 156.0), -ear, (0.28 * 175.0, 0.45 * 156.0), None);
    draw_part(pm, &sp.ear_r, gh, (0.0, 0.0, 175.0, 156.0), ear, (0.72 * 175.0, 0.45 * 156.0), None);
    draw_part(pm, &sp.headset, gh, (142.0, 97.0, 42.0, 40.0), mic, (142.0 + 21.0, 97.0 + 20.0), None);
}

/// 정지 멤버 한 프레임: 캐릭터 영역 높이에 맞춘 가운데 그림. bounce_px = 위아래(px).
pub fn still_frame(pm: &mut Pixmap, img: &Pixmap, cv: &Canvas, bounce_px: f32) {
    pm.fill(Color::TRANSPARENT);
    let s = cv.char_h / img.height() as f32;
    let x0 = cv.w / 2.0 - img.width() as f32 * s / 2.0;
    let y0 = cv.h - cv.char_h + bounce_px;
    draw_img(pm, img, Aff::t(x0, y0).mul(Aff::s(s, s)), None);
}

/// 겹침 표시: LIVE 배지(머리 위 가운데) · 새 트윗 점(오른쪽 위). scale = DPI 배율.
/// badge 는 LIVE 일 때만 넘긴다.
pub fn draw_overlays(
    pm: &mut Pixmap,
    badge: Option<&Pixmap>,
    unread: bool,
    cv: &Canvas,
    scale: f32,
) {
    if let Some(b) = badge {
        let (bw, bh) = (56.0 * scale, 22.0 * scale);
        let live_h = cv.h - cv.char_h;
        let t = Aff::t(cv.w / 2.0 - bw / 2.0, (live_h - bh) / 2.0)
            .mul(Aff::s(bw / b.width() as f32, bh / b.height() as f32));
        draw_img(pm, b, t, None);
    }
    if unread {
        let cx = cv.w / 2.0 + 50.0 * scale;
        let cy = cv.h - cv.char_h + 4.0 * scale;
        let Some(path) = PathBuilder::from_circle(cx, cy, 5.0 * scale) else {
            return;
        };
        let mut fill = Paint::default();
        fill.set_color_rgba8(0xff, 0x4f, 0x8b, 0xff);
        pm.fill_path(&path, &fill, FillRule::Winding, Transform::identity(), None);
        let mut edge = Paint::default();
        edge.set_color_rgba8(0xff, 0xff, 0xff, 0xff);
        let stroke = Stroke { width: 1.5 * scale, ..Stroke::default() };
        pm.stroke_path(&path, &edge, &stroke, Transform::identity(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::critter::motion::{self, Phase};

    #[test]
    fn aff_mul_applies_right_first() {
        // (1,1) → 스케일 2 → (2,2) → 이동 10 → (12,2)
        let a = Aff::t(10.0, 0.0).mul(Aff::s(2.0, 2.0));
        assert_eq!(a.apply(1.0, 1.0), (12.0, 2.0));
    }

    #[test]
    fn rotation_keeps_pivot() {
        let r = Aff::rot(90.0, 5.0, 5.0);
        let (x, y) = r.apply(5.0, 5.0);
        assert!((x - 5.0).abs() < 1e-4 && (y - 5.0).abs() < 1e-4);
        // 중심 오른쪽 한 칸 (6,5) → 90도 회전 → 아래 한 칸 (5,6)
        let (x, y) = r.apply(6.0, 5.0);
        assert!((x - 5.0).abs() < 1e-4 && (y - 6.0).abs() < 1e-4);
    }

    #[test]
    fn rig_bottom_on_canvas_bottom_center_and_flip() {
        let cv = Canvas { w: 140.0, h: 158.0, char_h: 130.0 };
        let g = rig_to_screen(&cv, 1.0, 0.0);
        let (x, y) = g.apply(120.0, 234.0);
        assert!((y - 158.0).abs() < 1e-3, "리그 바닥 = 창 아랫변");
        assert!((x - 70.0).abs() < 1e-3, "리그 x 120 = 창 가운데");
        // 좌우 반전: x 120 + 20 은 가운데에서 왼쪽으로 가야 한다
        let gl = rig_to_screen(&cv, -1.0, 0.0);
        let (xl, _) = gl.apply(140.0, 234.0);
        let (xr, _) = g.apply(140.0, 234.0);
        assert!(xr > 70.0 && xl < 70.0);
        assert!((xr - xl).abs() > 0.0 && (xr + xl - 140.0).abs() < 1e-3, "가운데 대칭");
    }

    #[test]
    fn walk_pose_is_deterministic() {
        assert_eq!(motion::pose(Phase::Walk, 0.3), motion::pose(Phase::Walk, 0.3));
    }

    /// 눈으로 확인용: target/critter_frame_*.png 를 만든다.
    /// 실행: cargo test critter::rig::tests::yuno_frame_png -- --ignored --nocapture
    #[test]
    #[ignore]
    fn yuno_frame_png() {
        let sp = YunoSprites::load().expect("유노 그림 해독");
        let badge = decode(live_badge_bytes()).expect("LIVE 배지 해독");
        let scale = 2.0f32;
        let (w, h) = (280u32, 316u32);
        let cv = Canvas { w: w as f32, h: h as f32, char_h: h as f32 - 28.0 * scale };
        let mut pm = Pixmap::new(w, h).expect("캔버스");
        let root = env!("CARGO_MANIFEST_DIR");

        // (이름, 포즈, 눈 감음, 좌우 배율, LIVE, 새 트윗)
        let cases: [(&str, Pose, bool, f32, bool, bool); 4] = [
            ("walk", motion::pose(Phase::Walk, 0.25), false, 1.0, false, false),
            ("walk_left", motion::pose(Phase::Walk, 0.6), false, -1.0, false, false),
            ("idle_blink", motion::pose(Phase::Idle, 0.8), true, 1.0, false, false),
            ("idle_live_unread", motion::pose(Phase::Idle, 1.6), false, 1.0, true, true),
        ];
        for (name, p, closed, flip, live, unread) in cases {
            yuno_frame(&mut pm, &sp, &p, closed, &cv, flip);
            let badge_ref = if live { Some(&badge) } else { None };
            draw_overlays(&mut pm, badge_ref, unread, &cv, scale);
            let path = format!("{root}/target/critter_frame_{name}.png");
            save_over_bg(&pm, &path);
            println!("saved {path}");
        }
    }

    /// 투명 부분을 연한 바탕 위에 펴서 PNG 로 저장한다(premultiplied 기준 합성).
    fn save_over_bg(pm: &Pixmap, path: &str) {
        let bg = [246u32, 238, 250];
        let mut out = Vec::with_capacity(pm.data().len());
        for px in pm.data().chunks_exact(4) {
            let a = px[3] as u32;
            for k in 0..3 {
                out.push((px[k] as u32 + bg[k] * (255 - a) / 255).min(255) as u8);
            }
            out.push(255);
        }
        let img = image::RgbaImage::from_raw(pm.width(), pm.height(), out).expect("PNG 버퍼");
        img.save(path).expect("PNG 저장");
    }
}
