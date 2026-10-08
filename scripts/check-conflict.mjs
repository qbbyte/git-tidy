import { parseConflict, assemble, countHunks } from "../src/lib/conflict.ts";

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

// git 默认的 merge 风格：没有 ||||||| 那一段
const mergeStyle = [
  "第一行",
  "<<<<<<< HEAD",
  "我方",
  "=======",
  "对方",
  ">>>>>>> side",
  "最后一行",
].join("\n");

const parsedMerge = parseConflict(mergeStyle);
check("merge 风格能解析出 1 块", countHunks(mergeStyle), 1);
// 每块前后各一段：单块时是 2 段，中间的空段不留（夹在两块之间）
check("干净片段", parsedMerge.clean, [["第一行"], ["最后一行"]]);
check("块内容", [parsedMerge.hunk[0].ours, parsedMerge.hunk[0].theirs], [["我方"], ["对方"]]);
check(
  "全取我",
  assemble(parsedMerge, ["ours"]),
  "第一行\n我方\n最后一行",
);
check(
  "全取对",
  assemble(parsedMerge, ["theirs"]),
  "第一行\n对方\n最后一行",
);

// diff3 风格：中间多一段共同祖先
const diff3Style = [
  "<<<<<<< HEAD",
  "我方",
  "||||||| merged common ancestors",
  "祖先",
  "=======",
  "对方",
  ">>>>>>> side",
].join("\n");
const parsedDiff3 = parseConflict(diff3Style);
check("diff3 风格有祖先段", parsedDiff3.hunk[0].base, ["祖先"]);
check(
  "取回祖先",
  assemble(parsedDiff3, ["base"]),
  "祖先",
);

// 多块：顺序不能乱，干净片段要留在原位
const twoHunks = [
  "头",
  "<<<<<<< HEAD",
  "A1",
  "=======",
  "A2",
  ">>>>>>> side",
  "中",
  "<<<<<<< HEAD",
  "B1",
  "=======",
  "B2",
  ">>>>>>> side",
  "尾",
].join("\n");
const parsedTwo = parseConflict(twoHunks);
check("两块的干净片段", parsedTwo.clean, [["头"], ["中"], ["尾"]]);
check("两块都取我", assemble(parsedTwo, ["ours", "ours"]), "头\nA1\n中\nB1\n尾");
check("第一块取我第二块取对", assemble(parsedTwo, ["ours", "theirs"]), "头\nA1\n中\nB2\n尾");

// 标记被手工改坏：必须报出来，不能拼一个半截结果
check("不成对的标记解析为 null", parseConflict("<<<<<<< HEAD\n只有我方\n"), null);
check("嵌套开标记解析为 null", parseConflict("<<<<<<< HEAD\n<<<<<<< HEAD\n=======\nA\n>>>>>>> side"), null);
check("没有标记时块数为 0", countHunks("干干净净的内容\n"), 0);

// 自定义正文：用户手写的整段原样替换那一块
check(
  "custom 原样替换",
  assemble(parsedMerge, ["custom"], ["我拼的"]),
  "第一行\n我拼的\n最后一行",
);

console.log(failed === 0 ? "\n全部通过" : `\n${failed} 条失败`);
process.exit(failed === 0 ? 0 : 1);
