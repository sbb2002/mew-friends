//! 유노 움직임 · 포즈 · 깜빡임 계산. 순수 함수(Win32 없음) — 명세 docs/IMPL_SPEC.md §7-4.
//! 각도 = 도(deg), bounce · head_y · mic = 리그 단위. 시간 t = 앱 시작 뒤 경과 초.

use std::f64::consts::PI;

/// 걷기 / 쉬기.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Walk,
    Idle,
}

/// 리그 포즈. 웹 원본 `bdayscene-yuno.js` 의 q 값과 같은 뜻.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Pose {
    pub bounce: f64,
    pub body_roll: f64,
    pub left: f64,
    pub right: f64,
    pub ear: f64,
    pub tail: f64,
    pub head_angle: f64,
    pub head_y: f64,
    pub mic: f64,
}

/// 상태와 시각 t 로 포즈를 정한다(명세 §7-4 식 그대로).
pub fn pose(phase: Phase, t: f64) -> Pose {
    match phase {
        Phase::Walk => {
            let s1 = (2.0 * PI * t / 1.0).sin();
            Pose {
                bounce: -(PI * t / 0.5).sin().abs() * 4.0,
                body_roll: s1 * 3.0,
                left: s1 * 12.0,
                right: -s1 * 12.0,
                ear: s1 * 6.0,
                tail: (2.0 * PI * t / 0.7).sin() * 10.0,
                head_angle: s1 * 2.0,
                head_y: 0.0,
                mic: 0.0,
            }
        }
        Phase::Idle => Pose {
            bounce: (2.0 * PI * t / 3.2).sin() * 1.5,
            ear: (2.0 * PI * t / 4.0).sin() * 3.0,
            tail: (2.0 * PI * t / 2.6).sin() * 6.0,
            ..Pose::default()
        },
    }
}

/// 깜빡임: 7.6초 주기 중 44.5%~46.3% 구간에 눈을 감는다(웹 원본과 같은 규칙).
pub fn blink_closed(t: f64) -> bool {
    let pct = (t + 0.7).rem_euclid(7.6) / 7.6 * 100.0;
    (44.5..46.3).contains(&pct)
}

/// 시드 있는 간단 LCG(새 크레이트 없이 무작위).
#[derive(Clone, Debug)]
pub struct Lcg(u64);

impl Lcg {
    pub fn new(seed: u64) -> Self {
        Lcg(seed | 1)
    }

    /// [0, 1) 균등 난수.
    pub fn next_f64(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    /// [lo, hi) 난수.
    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next_f64()
    }
}

/// 가로 이동 한 번. 벽(min · max)에 닿으면 멈추고 방향을 반대로 바꾼다.
/// 반환 = (새 x, 새 방향 ±1). max <= min 이면 min 에 고정.
pub fn advance(x: f64, dir: f64, dx: f64, min: f64, max: f64) -> (f64, f64) {
    if max <= min {
        return (min, dir);
    }
    let x = x.clamp(min, max);
    let nx = x + dir * dx;
    if nx >= max {
        (max, -1.0)
    } else if nx <= min {
        (min, 1.0)
    } else {
        (nx, dir)
    }
}

/// 걷기 ↔ 쉬기 무작위 반복 + 가로 위치.
#[derive(Clone, Debug)]
pub struct Wander {
    pub phase: Phase,
    phase_end: f64,
    pub x: f64,
    pub dir: f64,
    rng: Lcg,
}

impl Wander {
    /// 걷기로 시작한다(첫 걷기 6~15초).
    pub fn new(seed: u64, now: f64, x: f64) -> Self {
        let mut rng = Lcg::new(seed);
        let phase_end = now + rng.range(6.0, 15.0);
        Wander { phase: Phase::Walk, phase_end, x, dir: 1.0, rng }
    }

    /// 시각 now, 지난 시간 dt(초), 걷는 속도(px/초), 가로 범위.
    pub fn step(&mut self, now: f64, dt: f64, speed: f64, min: f64, max: f64) {
        if now >= self.phase_end {
            self.toggle(now);
        }
        if self.phase == Phase::Walk {
            let (x, dir) = advance(self.x, self.dir, speed * dt, min, max);
            self.x = x;
            self.dir = dir;
        }
    }

    /// 좌우 반전 배율: 왼쪽을 보면 -1.
    pub fn flip(&self) -> f64 {
        if self.dir < 0.0 { -1.0 } else { 1.0 }
    }

    fn toggle(&mut self, now: f64) {
        match self.phase {
            Phase::Walk => {
                self.phase = Phase::Idle;
                self.phase_end = now + self.rng.range(4.0, 10.0);
            }
            Phase::Idle => {
                self.phase = Phase::Walk;
                self.phase_end = now + self.rng.range(6.0, 15.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blink_window() {
        // pct = 45 이 되는 t: (t + 0.7) mod 7.6 = 3.42
        assert!(blink_closed(3.42 - 0.7));
        assert!(!blink_closed(0.0));
        assert!(!blink_closed(3.42 - 0.7 + 1.0));
        // 주기: 7.6초 뒤에도 같다
        assert!(blink_closed(3.42 - 0.7 + 7.6));
    }

    #[test]
    fn walk_pose_ranges() {
        for i in 0..200 {
            let t = i as f64 * 0.037;
            let p = pose(Phase::Walk, t);
            assert!(p.bounce <= 0.0 && p.bounce >= -4.0 - 1e-9);
            assert!((p.left + p.right).abs() < 1e-9, "좌우 팔은 반대 위상");
            assert!(p.left.abs() <= 12.0 + 1e-9);
            assert!(p.tail.abs() <= 10.0 + 1e-9);
        }
        // t = 0.25 에서 걸음 바운스는 최대(-4), 좌우 팔 최대 ≈ 12 (sin(π/2) 위상)
        let p = pose(Phase::Walk, 0.25);
        assert!((p.bounce + 4.0).abs() < 1e-9);
        assert!((p.left - 12.0).abs() < 1e-9);
    }

    #[test]
    fn idle_pose_is_gentle() {
        let p = pose(Phase::Idle, 0.8);
        assert!(p.left == 0.0 && p.right == 0.0 && p.body_roll == 0.0);
        assert!(p.bounce.abs() <= 1.5 + 1e-9);
        assert!(p.ear.abs() <= 3.0 + 1e-9);
    }

    #[test]
    fn wall_reverses_direction() {
        // 오른쪽 벽 근처에서 오른쪽으로 가면 벽에 붙고 방향이 반대로 바뀐다
        let (x, d) = advance(99.0, 1.0, 5.0, 0.0, 100.0);
        assert_eq!((x, d), (100.0, -1.0));
        // 왼쪽 벽
        let (x, d) = advance(1.0, -1.0, 5.0, 0.0, 100.0);
        assert_eq!((x, d), (0.0, 1.0));
        // 가운데에서는 그냥 간다
        let (x, d) = advance(50.0, -1.0, 5.0, 0.0, 100.0);
        assert_eq!((x, d), (45.0, -1.0));
        // 범위가 없으면 min 에 고정
        assert_eq!(advance(3.0, 1.0, 5.0, 10.0, 10.0), (10.0, 1.0));
    }

    #[test]
    fn wander_alternates_and_stays_in_bounds() {
        let mut w = Wander::new(42, 0.0, 10.0);
        assert_eq!(w.phase, Phase::Walk);
        let mut now = 0.0;
        let mut saw_idle = false;
        for _ in 0..(60 * 60 * 3) {
            now += 0.05;
            let before = w.x;
            w.step(now, 0.05, 40.0, 0.0, 300.0);
            assert!(w.x >= 0.0 && w.x <= 300.0);
            if w.phase == Phase::Idle {
                saw_idle = true;
                assert_eq!(w.x, before, "쉬는 동안 위치는 그대로");
            }
        }
        assert!(saw_idle, "3분 안에 쉬기 상태가 한 번은 와야 한다");
    }

    #[test]
    fn lcg_range_is_bounded() {
        let mut r = Lcg::new(7);
        for _ in 0..1000 {
            let v = r.range(4.0, 10.0);
            assert!((4.0..10.0).contains(&v));
        }
    }
}
