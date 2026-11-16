// 로그인 계열 화면 장식. 휴대폰 390x844, 넓은 화면 1280x880 장면 좌표 그대로.
// 하늘(위 고정)과 언덕(아래 고정)을 나눠 화면 비율이 달라도 해와 언덕이 잘리지 않음.
// 넓은 화면 언덕은 1280 장면 높이(290px)가 상한. 그보다 넓으면 언덕만 가로로 늘어남.
// 색은 --auth-* 변수. 별과 달 깎기 원은 다크에서만 보임.

import type { ReactNode } from 'react'

const STAR = 'fill-auth-star opacity-0 dark:opacity-85'
const MOON_BITE = 'fill-auth-sky opacity-0 dark:opacity-100'
const CLOUD = 'fill-auth-cloud dark:opacity-85'
const PLANT = 'fill-auth-plant stroke-auth-stem'

// 넓은 화면 풀 줄기 묶음. children 은 같은 좌표계의 곁줄기.
function Plant({
  transform,
  children,
}: {
  transform: string
  children?: ReactNode
}) {
  return (
    <g
      className={PLANT}
      transform={transform}
      strokeWidth="4"
      strokeLinecap="round"
    >
      <path d="M0 0 C 2 -40, 10 -84, 30 -124" fill="none" />
      <ellipse
        cx="-12"
        cy="-40"
        rx="8"
        ry="19"
        transform="rotate(-38 -12 -40)"
        stroke="none"
      />
      <ellipse
        cx="22"
        cy="-58"
        rx="8"
        ry="19"
        transform="rotate(34 22 -58)"
        stroke="none"
      />
      <ellipse
        cx="2"
        cy="-86"
        rx="7"
        ry="17"
        transform="rotate(-30 2 -86)"
        stroke="none"
      />
      <ellipse
        cx="34"
        cy="-108"
        rx="7"
        ry="16"
        transform="rotate(40 34 -108)"
        stroke="none"
      />
      {children}
    </g>
  )
}

// 하늘 장식. 휴대폰은 페이지 위(스크롤과 함께), 넓은 화면은 틀(relative) 전체.
// fixed 금지: 앱 셸 안 RouteError 렌더 시 사이드바를 덮음.
export function AuthBackdrop() {
  return (
    <>
      <svg
        aria-hidden="true"
        viewBox="0 0 390 320"
        className="pointer-events-none absolute right-0 top-0 w-full max-w-[480px] md:hidden"
      >
        <g className={STAR}>
          <circle cx="60" cy="90" r="1.6" />
          <circle cx="120" cy="60" r="1.2" />
          <circle cx="210" cy="110" r="1.4" />
          <circle cx="250" cy="70" r="1" />
          <circle cx="160" cy="200" r="1.1" />
          <circle cx="360" cy="220" r="1.3" />
          <circle cx="30" cy="260" r="1" />
          <circle cx="290" cy="300" r="1.2" />
        </g>
        <circle className="fill-auth-sun" cx="330" cy="128" r="40" />
        <circle className={MOON_BITE} cx="346" cy="116" r="36" />
        <g className={CLOUD}>
          <ellipse cx="58" cy="176" rx="56" ry="15" />
          <circle cx="40" cy="166" r="18" />
          <circle cx="70" cy="156" r="24" />
        </g>
      </svg>
      <div className="pointer-events-none absolute inset-0 hidden overflow-hidden md:block">
        <svg
          aria-hidden="true"
          viewBox="0 0 1280 480"
          className="absolute inset-x-0 top-0 w-full"
        >
          <g className={STAR}>
            <circle cx="140" cy="90" r="2" />
            <circle cx="320" cy="60" r="1.5" />
            <circle cx="460" cy="140" r="1.8" />
            <circle cx="620" cy="80" r="1.4" />
            <circle cx="780" cy="130" r="1.6" />
            <circle cx="900" cy="60" r="1.3" />
            <circle cx="1200" cy="120" r="1.8" />
            <circle cx="60" cy="220" r="1.4" />
            <circle cx="1220" cy="380" r="1.5" />
            <circle cx="380" cy="420" r="1.2" />
            <circle cx="960" cy="460" r="1.3" />
          </g>
          <circle className="fill-auth-sun" cx="1046" cy="176" r="64" />
          <circle className={MOON_BITE} cx="1072" cy="156" r="58" />
          <g className={CLOUD}>
            <ellipse cx="206" cy="336" rx="96" ry="26" />
            <circle cx="166" cy="320" r="30" />
            <circle cx="218" cy="302" r="42" />
            <circle cx="262" cy="326" r="24" />
          </g>
          <g className={CLOUD}>
            <ellipse cx="1096" cy="268" rx="84" ry="22" />
            <circle cx="1062" cy="254" r="26" />
            <circle cx="1106" cy="238" r="36" />
            <circle cx="1146" cy="258" r="20" />
          </g>
        </svg>
        <svg
          aria-hidden="true"
          viewBox="0 590 1280 290"
          preserveAspectRatio="none"
          className="absolute inset-x-0 bottom-0 aspect-[1280/290] max-h-[290px] w-full"
        >
          <path
            className="fill-auth-hill-1"
            d="M0 690 C 160 610, 320 630, 450 696 S 780 770, 920 690 S 1170 596, 1280 632 L1280 880 L0 880 Z"
          />
          <path
            className="fill-auth-hill-2"
            d="M0 782 C 200 716, 390 738, 570 792 S 910 822, 1090 762 S 1240 730, 1280 742 L1280 880 L0 880 Z"
          />
          <path
            className="fill-auth-hill-3"
            d="M0 846 C 260 806, 520 832, 760 852 S 1100 822, 1280 834 L1280 880 L0 880 Z"
          />
        </svg>
        {/* 풀은 언덕과 따로 두어 늘어나지 않음. 위치는 1280 장면 비율, 크기는 언덕과 같은 상한. */}
        <svg
          aria-hidden="true"
          viewBox="80 700 140 180"
          className="absolute bottom-0 left-[6.25%] w-[10.9375%] max-w-[140px]"
        >
          <Plant transform="translate(118 838)">
            <path d="M60 4 C 62 -20, 66 -40, 78 -60" fill="none" />
            <ellipse
              cx="54"
              cy="-24"
              rx="6"
              ry="14"
              transform="rotate(-36 54 -24)"
              stroke="none"
            />
            <ellipse
              cx="80"
              cy="-38"
              rx="6"
              ry="14"
              transform="rotate(36 80 -38)"
              stroke="none"
            />
          </Plant>
        </svg>
        <svg
          aria-hidden="true"
          viewBox="1080 700 140 180"
          className="absolute bottom-0 right-[4.6875%] w-[10.9375%] max-w-[140px]"
        >
          <Plant transform="translate(1162 846) scale(-1 1)" />
        </svg>
      </div>
    </>
  )
}

// 휴대폰 언덕. 본문 뒤 흐름이라 내용이 길면 스크롤 끝에 붙음.
export function AuthHills() {
  return (
    <svg
      aria-hidden="true"
      data-auth-hills=""
      viewBox="0 670 390 174"
      className="pointer-events-none mt-auto block w-full shrink-0 md:hidden"
    >
      <path
        className="fill-auth-hill-1"
        d="M0 708 C 80 676, 150 684, 210 712 S 330 736, 390 700 L390 844 L0 844 Z"
      />
      <path
        className="fill-auth-hill-2"
        d="M0 770 C 90 742, 190 752, 270 774 S 360 778, 390 766 L390 844 L0 844 Z"
      />
      <path
        className="fill-auth-hill-3"
        d="M0 818 C 120 800, 260 810, 390 806 L390 844 L0 844 Z"
      />
      <g className="fill-auth-bush">
        <circle cx="58" cy="764" r="16" />
        <circle cx="80" cy="756" r="20" />
        <circle cx="102" cy="766" r="13" />
      </g>
      <g className="fill-auth-bush">
        <circle cx="304" cy="762" r="14" />
        <circle cx="324" cy="755" r="18" />
        <circle cx="344" cy="764" r="12" />
      </g>
      <g
        className={PLANT}
        transform="translate(26 830)"
        strokeWidth="3.2"
        strokeLinecap="round"
      >
        <path d="M0 0 C 2 -30, 8 -62, 22 -90" fill="none" />
        <ellipse
          cx="-8"
          cy="-30"
          rx="6"
          ry="14"
          transform="rotate(-38 -8 -30)"
          stroke="none"
        />
        <ellipse
          cx="17"
          cy="-44"
          rx="6"
          ry="14"
          transform="rotate(34 17 -44)"
          stroke="none"
        />
        <ellipse
          cx="3"
          cy="-64"
          rx="5"
          ry="12"
          transform="rotate(-30 3 -64)"
          stroke="none"
        />
        <ellipse
          cx="26"
          cy="-80"
          rx="5"
          ry="12"
          transform="rotate(40 26 -80)"
          stroke="none"
        />
      </g>
      <g
        className={PLANT}
        transform="translate(368 834) scale(-1 1)"
        strokeWidth="3.2"
        strokeLinecap="round"
      >
        <path d="M0 0 C 2 -26, 8 -52, 20 -76" fill="none" />
        <ellipse
          cx="-7"
          cy="-26"
          rx="5"
          ry="12"
          transform="rotate(-38 -7 -26)"
          stroke="none"
        />
        <ellipse
          cx="15"
          cy="-40"
          rx="5"
          ry="12"
          transform="rotate(34 15 -40)"
          stroke="none"
        />
        <ellipse
          cx="22"
          cy="-66"
          rx="5"
          ry="11"
          transform="rotate(40 22 -66)"
          stroke="none"
        />
      </g>
    </svg>
  )
}
