<script setup lang="ts">
import { computed } from "vue";
import { NAlert, NButton, NEmpty, NSpin } from "naive-ui";
import { formatBytes, formatDelta } from "@/format";
import { GitTidyError } from "@/api/client";
import type { Diff, Hunk, Line, LineKind, Blob as ImageBlob } from "@/api/detail";

/**
 * 单个文件的差异。四种降级都在这里分派，Rust 侧已经把"该画哪一种"判完了
 * （`Render`），这里不再按长度自己猜——两处各判一套，早晚会对不上。
 */
const props = defineProps<{
  diff: Diff | null;
  loading: boolean;
  error: GitTidyError | null;
}>();

const emit = defineEmits<{ retry: [] }>();

interface Piece {
  text: string;
  /** true 才上底色：改过的那一小段，前后没动的内容是灰的 */
  changed: boolean;
}

type Row =
  | {
      type: "hunk";
      key: string;
      oldRange: string;
      newRange: string;
      header: string;
    }
  | {
      type: "line";
      key: string;
      kind: LineKind;
      oldNo: number | null;
      newNo: number | null;
      pieces: Piece[];
    };

const rows = computed<Row[]>(() => {
  const diff = props.diff;
  if (diff === null || diff.render !== "text") return [];
  const out: Row[] = [];
  diff.hunks.forEach((hunk, index) => collect(out, hunk, index));
  return out;
});

/**
 * 一段 hunk 里的行。连续的"删若干 + 增若干"按位置配对，配对成功的两侧各取词级公共
 * 前后缀，只给中间那段上底色；配不上的（数量不等）整行高亮。
 *
 * 这是在前端就地算的，不再起第二个 git 进程要 `--word-diff`：那条路的代价是每次点开
 * 多一个进程，而词级高亮只是"看得更顺眼"，不值这个钱。
 */
function collect(out: Row[], hunk: Hunk, hunkIndex: number) {
  out.push({
    type: "hunk",
    key: `h${hunkIndex}`,
    oldRange: range(hunk.oldStart, hunk.oldCount),
    newRange: range(hunk.newStart, hunk.newCount),
    header: hunk.header,
  });

  const lines = hunk.lines;
  let i = 0;
  while (i < lines.length) {
    if (lines[i].kind !== "delete") {
      out.push(whole(lines[i], `${hunkIndex}-${i}`));
      i += 1;
      continue;
    }
    const dels = takeRun(lines, i, "delete");
    const adds = takeRun(lines, dels.next, "add");
    const deleted = dels.picked.filter((line) => line.kind === "delete");
    const added = adds.picked.filter((line) => line.kind === "add");
    // 数量不等就不配：把第 3 行删除和第 1 行新增涂成同一处改动是编故事
    const paired = deleted.length === added.length && deleted.length > 0;

    let d = 0;
    dels.picked.forEach((line, offset) => {
      const key = `${hunkIndex}-${dels.start + offset}`;
      if (line.kind !== "delete") {
        out.push(whole(line, key));
        return;
      }
      out.push(pieces(line, paired ? added[d] : null, key));
      d += 1;
    });
    let a = 0;
    adds.picked.forEach((line, offset) => {
      const key = `${hunkIndex}-${adds.start + offset}`;
      if (line.kind !== "add") {
        out.push(whole(line, key));
        return;
      }
      out.push(pieces(line, paired ? deleted[a] : null, key));
      a += 1;
    });
    i = adds.next;
  }
}

/** 取连续的同向行。`\` 那一句（没有末尾换行）挂在它前面那行后面，不打断这一段 */
function takeRun(lines: Line[], from: number, kind: LineKind) {
  const picked: Line[] = [];
  let i = from;
  while (i < lines.length) {
    const line = lines[i];
    if (line.kind === kind || (line.kind === "meta" && picked.length > 0)) {
      picked.push(line);
      i += 1;
      continue;
    }
    break;
  }
  return { picked, start: from, next: i };
}

function whole(line: Line, key: string): Row {
  return {
    type: "line",
    key,
    kind: line.kind,
    oldNo: line.oldNo,
    newNo: line.newNo,
    // 上下文和 `\` 都不上底色。删/增单独出现时整行就是改动本身
    pieces:
      line.kind === "context" || line.kind === "meta"
        ? [{ text: line.text, changed: false }]
        : [{ text: line.text, changed: true }],
  };
}

function pieces(line: Line, other: Line | null, key: string): Row {
  return {
    type: "line",
    key,
    kind: line.kind,
    oldNo: line.oldNo,
    newNo: line.newNo,
    pieces: other === null ? [{ text: line.text, changed: true }] : trim(line.text, other.text),
  };
}

/** 词级公共前后缀留给两侧各自涂灰，中间那截才是这次真的改了的 */
function trim(own: string, other: string): Piece[] {
  const a = tokenize(own);
  const b = tokenize(other);
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head += 1;
  let tail = 0;
  while (
    head + tail < a.length &&
    head + tail < b.length &&
    a[a.length - 1 - tail] === b[b.length - 1 - tail]
  ) {
    tail += 1;
  }

  const out: Piece[] = [];
  if (head > 0) out.push({ text: a.slice(0, head).join(""), changed: false });
  if (a.length - tail > head) {
    out.push({ text: a.slice(head, a.length - tail).join(""), changed: true });
  }
  if (tail > 0) out.push({ text: a.slice(a.length - tail).join(""), changed: false });
  return out;
}

/**
 * 按空白切词。**中文切不出来**：一段没有空格的话整段当一个词，那这行就整行高亮，
 * 和不做词级一样。真要做到字级得引分词器，为一个高亮颜色不值当。
 */
function tokenize(text: string): string[] {
  return text.split(/(\s+)/).filter((piece) => piece !== "");
}

function range(start: number, count: number): string {
  // 0 行是 git 表示"这一侧没有内容"的写法（新增/删除文件），不是从第 0 行开始
  return count === 0 ? `${start},0` : `${start}–${start + count - 1}`;
}

function dataUrl(blob: ImageBlob | null): string {
  return blob === null ? "" : `data:${blob.mime};base64,${blob.base64}`;
}

const render = computed(() => props.diff?.render ?? null);
const filePath = computed(() => props.diff?.path ?? "");
const lineCount = computed(() => props.diff?.lineCount ?? 0);
const byteCount = computed(() => props.diff?.byteCount ?? 0);
const added = computed(() => props.diff?.added ?? 0);
const deleted = computed(() => props.diff?.deleted ?? 0);
const oldSize = computed(() => props.diff?.oldSize ?? null);
const newSize = computed(() => props.diff?.newSize ?? null);
const oldImage = computed(() => props.diff?.images?.old ?? null);
const newImage = computed(() => props.diff?.images?.new ?? null);

/**
 * 上面这一串 `?? ` 兜底不是防御性编程，是为了模板里的类型：prop 是 `Diff | null`，
 * 而"这一段只在非 null 时才渲染"这件事模板条件表达不出来（vue-tsc 认不认得到那层
 * 嵌套全看代码生成怎么写）。宁可多写几行确定的，也不赌它。
 */
const stats = computed(() => {
  if (props.diff === null) return "";
  const sizes = formatDelta(oldSize.value, newSize.value);
  return sizes === "" ? "" : `｜${sizes}`;
});
</script>

<template>
  <div class="diff">
    <n-alert
      v-if="error"
      type="error"
      :title="error.message"
      class="block"
    >
      <div>错误码：{{ error.code }}</div>
      <pre v-if="error.detail" class="raw-output">{{ error.detail }}</pre>
      <n-button size="tiny" @click="emit('retry')">重试</n-button>
    </n-alert>

    <div v-else-if="loading" class="waiting">
      <n-spin size="small" />
      <span class="muted">正在取这个文件的差异…</span>
    </div>

    <template v-else-if="diff">
      <!--
        图片只并排看。滑动对比和差异叠加要的是同一份字节摆两次，等这个视图用顺了再加，
        现在先不占这块地方（§7.5 的取舍）。
      -->
      <div v-if="render === 'image'" class="images">
        <figure class="image">
          <figcaption>改动前</figcaption>
          <img v-if="oldImage" :src="dataUrl(oldImage)" :alt="filePath" />
          <span v-else class="muted">这一侧没有文件</span>
          <div class="muted">{{ formatBytes(oldSize) }}</div>
        </figure>
        <figure class="image">
          <figcaption>改动后</figcaption>
          <img v-if="newImage" :src="dataUrl(newImage)" :alt="filePath" />
          <span v-else class="muted">这一侧没有文件</span>
          <div class="muted">{{ formatBytes(newSize) }}</div>
        </figure>
      </div>

      <div v-else-if="render === 'binary'" class="notice">
        二进制文件，不能按行比。{{ stats }}
      </div>

      <div v-else-if="render === 'toolarge'" class="notice">
        改动 {{ lineCount }} 行 / {{ formatBytes(byteCount) }}，超出一次显示的闸门，
        只给统计：+{{ added }} −{{ deleted }}。
        <div class="muted">要看全文请在终端里 diff，或把它拆成几次提交。</div>
      </div>

      <n-empty
        v-else-if="render === 'empty'"
        description="没有逐行差异（忽略空白之后两侧内容相同）"
        class="block"
      />

      <div v-else class="body">
        <template v-for="row in rows" :key="row.key">
          <div v-if="row.type === 'hunk'" class="hunk">
            <span class="hunk-range">旧 {{ row.oldRange }}　新 {{ row.newRange }}</span>
            <span v-if="row.header" class="hunk-name">{{ row.header }}</span>
          </div>
          <div v-else class="line" :class="row.kind">
            <span class="no">{{ row.oldNo ?? "" }}</span>
            <span class="no">{{ row.newNo ?? "" }}</span>
            <span class="code"
              ><span
                v-for="(piece, index) in row.pieces"
                :key="index"
                :class="{ mark: piece.changed }"
                >{{ piece.text }}</span
              ></span
            >
          </div>
        </template>
      </div>
    </template>

    <div v-else class="notice muted">点左边清单里的一个文件看它改了什么。</div>
  </div>
</template>

<style scoped>
.diff {
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: auto;
  font-size: 12px;
}

.block {
  margin: 8px;
}

.waiting {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 16px;
}

.notice {
  padding: 12px 16px;
  line-height: 1.7;
}

.body {
  font-family: Consolas, "Courier New", monospace;
}

.hunk {
  display: flex;
  gap: 10px;
  padding: 3px 8px;
  background: #eef2f7;
  color: #4a5568;
  border-top: 1px solid #dfe4ec;
  border-bottom: 1px solid #dfe4ec;
}

.hunk-name {
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}

.line {
  display: flex;
  align-items: flex-start;
  white-space: pre-wrap;
  word-break: break-all;
  line-height: 18px;
}

.no {
  flex: none;
  width: 34px;
  text-align: right;
  color: #98a2b3;
  user-select: none;
  background: #fafbfc;
  border-right: 1px solid #eef1f5;
}

.code {
  padding: 0 8px;
  flex: 1;
  min-width: 0;
}

.line.add {
  background: #e6f6ea;
}

.line.delete {
  background: #fdeceb;
}

.line.meta {
  color: #98a2b3;
  font-style: italic;
}

/* 改过的词再深一档：整行已经上了底色，只有这一段才是真动了的 */
.mark {
  background: rgba(255, 180, 0, 0.35);
  border-radius: 2px;
}

.line.add .mark {
  background: rgba(46, 160, 67, 0.28);
}

.line.delete .mark {
  background: rgba(203, 36, 49, 0.22);
}

.images {
  display: flex;
  gap: 12px;
  padding: 12px;
  flex-wrap: wrap;
}

.image {
  margin: 0;
  padding: 8px;
  background: #fff;
  border: 1px solid #e5e8ee;
  border-radius: 6px;
}

.image img {
  display: block;
  max-width: 280px;
  max-height: 220px;
  /* 图片本身可能有透明区域，棋盘底才看得出alpha */
  background: repeating-conic-gradient(#f0f2f5 0% 25%, #fff 0% 50%) 50% / 16px 16px;
}

.image figcaption {
  font-size: 11px;
  color: #6b7484;
  margin-bottom: 6px;
}

.muted {
  color: #6b7484;
}

.raw-output {
  margin: 8px 0;
  white-space: pre-wrap;
  font-size: 12px;
  opacity: 0.8;
}
</style>
