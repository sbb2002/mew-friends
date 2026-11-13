//! 바탕화면 캐릭터 창. 명세: docs/IMPL_SPEC.md §7. [담당: 작업 C]

pub struct Critters {
    _private: (),
}

impl Critters {
    /// 멤버마다 창 하나. 메인 스레드에서 만든다.
    pub fn new(order: &[String]) -> Result<Critters, String> {
        let _ = order;
        todo!("작업 C")
    }

    /// 시계 기준으로 움직임을 진행하고 필요하면 다시 그린다.
    pub fn tick(&mut self) {
        todo!("작업 C")
    }

    /// 다음 tick 까지 원하는 간격(ms).
    pub fn frame_interval_ms(&self) -> u32 {
        todo!("작업 C")
    }

    pub fn set_live(&mut self, member: &str, live: bool) {
        let _ = (member, live);
        todo!("작업 C")
    }

    pub fn set_unread(&mut self, member: &str, unread: bool) {
        let _ = (member, unread);
        todo!("작업 C")
    }

    /// 지난 호출 뒤 눌린 캐릭터(멤버 key).
    pub fn take_clicks(&mut self) -> Vec<String> {
        todo!("작업 C")
    }
}
