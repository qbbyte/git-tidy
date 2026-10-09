<script setup lang="ts">
import { computed } from "vue";
import { NModal } from "naive-ui";
import RefPanel from "@/components/RefPanel.vue";

/**
 * 引用弹窗：分支 / 远程 / 标签。
 *
 * 原来它是主区右侧的第三栏（一条可拖宽的常驻 aside）。问题出在宽度上：窗口逻辑宽
 * 只有 ~1030px，展开那一栏后主区被挤到 400px 上下、历史列表只剩 ~160px，
 * 而分支与标签都属于"要用时才看"的东西，不值得常驻一份宽度。
 *
 * 改成弹窗之后：一分宽度不占，列表要滚多少滚多少；并且与「新建分支」的交互对齐——
 * 两者都是"打开 → 做完 → 关掉"，不再是一个常驻栏 + 一个弹窗混着。
 *
 * 内容完全复用 `RefPanel`（一行没改），它自带全部样式与写操作，
 * 所以这里只负责给一个不必挤宽度的容器。
 *
 * 关法：点遮罩、按 Esc、点右上角，都关。里面的写操作各有自己的确认（强推 / 删除
 * 都带二次确认），所以「手滑点到遮罩」不会直接造成改动，不必为此把遮罩锁死——
 * 锁死反而让人以为弹窗卡住了。
 */
const props = defineProps<{ show: boolean }>();

const emit = defineEmits<{
  (e: "update:show", value: boolean): void;
  /**
   * 面板里的「新建分支」。往上抛一层，由 App 去开 `NewBranchModal`——
   * 弹窗不嵌套弹窗，保持"所有弹窗都挂在 App"这一条不变。
   */
  (e: "new-branch"): void;
}>();

const visible = computed({
  get: () => props.show,
  set: (value: boolean) => emit("update:show", value),
});
</script>

<template>
  <n-modal
    v-model:show="visible"
    preset="card"
    title="引用"
    class="refs-modal"
    :bordered="false"
  >
    <!-- 分支几百条的仓库是常态：内容自己滚，弹窗不跟着长到屏幕外 -->
    <div class="refs-modal-body">
      <ref-panel @new-branch="emit('new-branch')" />
    </div>
  </n-modal>
</template>

<style scoped>
.refs-modal-body {
  max-height: 68vh;
  overflow-x: hidden;
  overflow-y: auto;
  /* 滚动条不贴着卡片边缘，留一点呼吸 */
  padding-right: 4px;
}
</style>

<style>
/* 弹窗卡片在 body 里，scoped 样式盖不到，宽度得在全局里定 */
.n-card.refs-modal {
  width: 560px;
  max-width: 92vw;
}
</style>
