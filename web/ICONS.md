# 아이콘 생성

원본은 `public/icon.svg`(둥근 모서리, 파비콘과 일반 아이콘)와 `public/icon-maskable.svg`(모서리 없는 정사각형, maskable 아이콘) 두 개입니다. PNG 는 `web/public/` 디렉터리로 이동한 뒤(`cd web/public`) 아래 명령으로 다시 만듭니다. 결과는 같은 디렉터리에 생깁니다.

```sh
docker run --rm -v "$PWD":/w -w /w debian:trixie-slim sh -c '
  apt-get update -qq && apt-get install -y -qq --no-install-recommends librsvg2-bin &&
  rsvg-convert -w 192 -h 192 icon.svg -o icon-192.png &&
  rsvg-convert -w 512 -h 512 icon.svg -o icon-512.png &&
  rsvg-convert -w 512 -h 512 icon-maskable.svg -o icon-maskable-512.png &&
  rsvg-convert -w 180 -h 180 --background-color "#FF9E57" icon.svg -o apple-touch-icon.png &&
  chown '"$(id -u):$(id -g)"' *.png'
```

- 얼굴이 아래로 잘리는 그림이라 maskable 은 별도 원본을 씁니다. 바탕이 끝까지 차고, 눈, 볼, 입, 머리카락은 안전 영역(지름 80% 원) 안에 들어갑니다. `icon.svg` 를 축소하면 잘린 선이 아이콘 안에 떠 보입니다.
- apple-touch-icon: iOS 는 투명 영역을 검게 칠하므로 모서리까지 바탕색으로 채웁니다. iOS 가 깎는 모서리 반지름이 `icon.svg` 와 비슷해 얼굴 잘림이 그대로 보입니다.
- 확인한 버전: rsvg-convert 2.60.0
