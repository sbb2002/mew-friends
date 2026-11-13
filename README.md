# Mew Friends

무겐다이 뮤타입(夢限大みゅーたいぷ) **비공식 팬 제작** Windows 바탕화면 친구 — MVP.

- 바탕화면(다른 창 뒤)에 멤버 동물 캐릭터가 산다. 유노는 걸어 다니고, 나머지는 아직 제자리.
- 방송이 시작되면 윈도우 토스트로 알린다(누르면 방송 페이지).
- 새 번역 트윗이 생기면 토스트로 알린다. 캐릭터를 누르면 그 멤버의 트윗 패널이 열린다.
- 데이터 = [mewtype-scheduler](https://mewtype-schduler.vercel.app/) 의 공개 JSON(읽기만).

> 비공식 팬 제작 · AI 생성 일러스트 · 공식과 무관. 캐릭터 그림은 공식 「ゆめみた animals」 를 참고해 AI 로 새로 그린 팬 제작물이다.

## 빌드

```powershell
cargo build --release   # target\release\mew_friends.exe
cargo test
```

## 실행

- `targetelease\mew_friends.exe` 를 실행하면 트레이에 아이콘이 생기고 바탕화면 아래쪽에 캐릭터가 나온다. 종료 · 알림 설정 · 시작 프로그램 등록은 트레이 메뉴에서.
- 처음 실행할 때는 이미 있던 방송 · 트윗을 알리지 않는다(이후 새로 생긴 것만).
- 토스트 확인: 앱이 꺼진 상태에서 `mew_friends.exe --test-toast`.
- 저장 위치: `%APPDATA%\mew_friends\`(state.json · 토스트 아이콘).

문서: `docs/PLAN.md`(기획 · 검증 기록) · `docs/IMPL_SPEC.md`(구현 명세).
