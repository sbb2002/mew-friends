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

문서: `docs/PLAN.md`(기획) · `docs/IMPL_SPEC.md`(구현 명세).
