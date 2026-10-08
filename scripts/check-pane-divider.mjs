// 分隔条的拖拽区间算得对不对。
//
// 这是纯算术，所以能测；而它一旦算错，症状是「某个窗口宽度下拖不动 /
// 被按到一边去」，在开发时很难联想到是这段 clamp。
import { clampValue, computePaneLimit } from "../src/composables/usePaneDivider.ts";

let failed = 0;

function check(name, actual, expected) {
  const a = JSON.stringify(actual);
  const e = JSON.stringify(expected);
  if (a !== e) {
    failed += 1;
    console.log(`FAIL ${name}\n  实际 ${a}\n  期望 ${e}`);
  } else {
    console.log(`ok   ${name}`);
  }
}

// clampValue 的三种情形
check("落在区间内不动", clampValue(0.4, 0.2, 0.7), 0.4);
check("低于下界抬到下界", clampValue(0.1, 0.2, 0.7), 0.2);
check("高于上界压到上界", clampValue(0.9, 0.2, 0.7), 0.7);
check("min 越过 max 时给中点", clampValue(0.5, 0.6, 0.4), 0.5);

// 历史页：比例制，详情栏（补集）至少 380px
const history = { unit: "ratio", min: 0.2, max: 0.7, containerMinWidth: 380 };
const at = (w) => computePaneLimit({ containerWidth: w, ...history });

check("宽屏 1920：区间不受补集约束", at(1920), { lo: 0.2, hi: 0.7 });
// 380/1280 = 0.296875，用容差比硬写小数好
check(
  "常见 1280：补集下限约 0.297",
  Math.abs(at(1280).lo - 380 / 1280) < 1e-9,
  true,
);
check("内容区 950：区间 [0.4, 0.6]", at(950), { lo: 0.4, hi: 0.6 });

// 窗口最窄（960 窗口 - 280 侧栏 - 内边距 ≈ 640）时两个下限会互相越过
const narrow = at(640);
const narrowEffective = { lo: clampValue(narrow.lo, narrow.lo, narrow.hi), hi: clampValue(narrow.hi, narrow.lo, narrow.hi) };
check("窄到两个下限互越时给 50/50", narrowEffective, { lo: 0.5, hi: 0.5 });

// 文件页与侧栏：像素制，补集下限不参与
const px = { unit: "px", min: 220, max: 520, containerMinWidth: 380 };
check("像素制不随容器缩放", computePaneLimit({ containerWidth: 640, ...px }), { lo: 220, hi: 520 });
check("像素制在宽屏上也不变", computePaneLimit({ containerWidth: 2560, ...px }), { lo: 220, hi: 520 });

// 容器还没布局出来（宽度 0）时不能除出 NaN
const zero = computePaneLimit({ containerWidth: 0, ...history });
check("容器宽为 0 时回落到 [min,max]", zero, { lo: 0.2, hi: 0.7 });
check("宽为 0 的取值不是 NaN", Number.isNaN(clampValue(0.5, zero.lo, zero.hi)), false);

console.log(failed === 0 ? "\n全部通过" : `\n${failed} 条失败`);
process.exit(failed === 0 ? 0 : 1);