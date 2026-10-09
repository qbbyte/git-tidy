// diff 语法高亮器：唯一不能破的规矩是**一个字都不能少**。
//
// 高亮只是观感，判错顶多是颜色不对；但一旦 tokenizer 吃掉字符，
// diff 就在骗人——而“少了一个字符”这种错误在界面上极难肉眼发现。
// 所以这里把“不丢字”当作硬约束逐行验证，并顺带看几类 token 判得对不对。
import { detectLang, highlight } from "../src/lib/highlight.ts";

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

/** 拼回去必须等于原文——这一条对下面所有样本都成立 */
function checkRoundTrip(line, lang) {
  const joined = highlight(line, lang)
    .map((token) => token.text)
    .join("");
  check(`不丢字（${lang}）：${line.slice(0, 28)}`, joined, line);
}

const SAMPLES = {
  ts: [
    'const answer: string = "hi"; // note',
    "export function pick<T>(list: T[], i = 0): T { return list[i]; }",
    "/* 块注释 */ const x = 0x1f + 1e3;",
    "`模板 ${value} 字符串`",
    "if (a && b) { return null } else { throw new Error('x') }",
  ],
  rust: [
    "pub fn main() -> Result<(), Box<dyn Error>> {",
    "    let v: Vec<String> = vec![\"a\".to_string()];",
    "    // 行注释",
    "    match v.first() { Some(x) => x, None => \"\" }",
  ],
  shell: [
    "#!/bin/sh",
    "cd \"$1\" && rm -rf -- \"$TMP/x\"",
    "for f in *.txt; do echo \"$f\"; done",
  ],
  css: [".card { color: #fff; margin: 0 auto; }", "/* 说明 */ a:hover { --x: 1px; }"],
  html: ["<div class=\"a\">文本 &amp; 1</div>", "<!-- 注释 -->"],
  json: ['{ "a": 1, "b": [true, null] }'],
  yaml: ["name: ci\non: push"],
  toml: ["[tool]\nvalue = 42 # 注释"],
  vue: ["<template><div/></template>", "const emit = defineEmits<{ (e: Event): void }>();"],
  text: ["任何字符都不用管", "{}", "\\"],
};

for (const [lang, lines] of Object.entries(SAMPLES)) {
  for (const line of lines) checkRoundTrip(line, lang);
}

// 空行、空串、以及只有空白的一行
checkRoundTrip("", "ts");
checkRoundTrip("    ", "rust");
check("空行不分词", highlight("", "ts"), []);

// 扩展名识别：认不出来的就老实说 text，不要瞎猜
check("按扩展名识别 ts", detectLang("src/stores/write.ts"), "ts");
check("带路径的 vue", detectLang("src/components/HookPanel.vue"), "vue");
check("大小写不敏感", detectLang("README.MD"), "md");
check("无扩展名", detectLog("Makefile"), "text");
check("未知扩展名", detectLang("a.bin"), "text");

function detectLog(path) {
  return detectLang(path);
}

// 几处判定抽查：启发判错只是颜色不对，但明显的该判对
const kinds = (line, lang) => highlight(line, lang).map((t) => `${t.kind}:${t.text}`);
check("字符串", kinds('const a = "x";', "ts").includes("string:\"x\""), true);
check("注释", kinds("const a = 1; // 说明", "ts").includes("comment:// 说明"), true);
check("数字", kinds("const a = 0x1f;", "ts").includes("number:0x1f"), true);
check("关键字", kinds("const a = 1;", "ts").includes("keyword:const"), true);
check("调用", kinds("render(path);", "ts").includes("function:render"), true);
check("类型", kinds("let x: Spec;", "ts").includes("type:Spec"), true);
check(
  "Shell 变量（引号外）",
  kinds("cd $HOME/x", "shell").includes("keyword:$HOME"),
  true,
);
// 引号里的 $1 整体是字符串——这是对的：引号就是告诉 shell “别展开”
check(
  "引号内不拆变量",
  kinds('cd "$1"', "shell").some((t) => t.startsWith("string:\"$1\"")),
  true,
);
check("JSON 键", kinds('{ "a": 1 }', "json").includes("type:\"a\""), true);

if (failed > 0) {
  console.log(`\n${failed} 项未通过`);
  process.exit(1);
}
console.log("\n全部通过");