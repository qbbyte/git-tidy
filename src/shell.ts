/**
 * 外壳动作的注册表（需求 7.23）。
 *
 * 快捷键定义在 `shortcuts.ts` 里（集中一处），但"按下去要做什么"只有当前页面知道：
 * 历史页的搜索框和提交页的搜索框不是同一个元素。所以这里放一个极小的注册表：
 * 页面挂载时登记自己负责的动作，外壳按键时找当前登记的那个。
 *
 * 为什么不用事件总线（`window.dispatchEvent`）：那样每个页面都要自己解析事件名、
 * 自己判断该不该响应，错了只会静默失效。注册表在名字写错时直接查不到，
 * 而且页面卸载时能注销，不会留一个指向已销毁组件的函数。
 */
export type ShellAction = "refresh" | "focusSearch" | "submit" | "switchRepo";

type Handler = () => void;

const handlers = new Map<ShellAction, Handler>();

/** 页面挂载时登记。返回值是注销函数，放进 `onUnmounted`。 */
export function provideShell(action: ShellAction, handler: Handler): () => void {
  handlers.set(action, handler);
  return () => {
    // 只删自己登记的那一个：页面切换有先后，删掉别人的登记会让快捷键突然失灵
    if (handlers.get(action) === handler) handlers.delete(action);
  };
}

/**
 * 执行一个动作。没有任何页面登记时返回 false——
 * 调用方据此决定"要不要给点反馈"，而不是静默什么都不发生。
 */
export function runShell(action: ShellAction): boolean {
  const handler = handlers.get(action);
  if (!handler) return false;
  handler();
  return true;
}