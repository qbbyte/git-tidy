import { invoke } from "@tauri-apps/api/core";

/** 与 Rust 侧 error.rs 中 GitError 的序列化结构一一对应 */
interface ErrorPayload {
  code: string;
  message: string;
  detail?: string;
}

export class GitTidyError extends Error {
  constructor(
    public readonly code: string,
    message: string,
    public readonly detail?: string,
  ) {
    super(message);
    this.name = "GitTidyError";
  }
}

/**
 * 所有 IPC 调用的唯一出口。Rust 侧抛出的错误 payload 在这里归一成 GitTidyError，
 * 业务层因此只需要 catch 一种错误类型，也不必关心 IPC 传输细节。
 */
export async function call<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (raw) {
    throw normalize(raw);
  }
}

function normalize(raw: unknown): GitTidyError {
  if (typeof raw === "object" && raw !== null && "code" in raw) {
    const payload = raw as ErrorPayload;
    return new GitTidyError(payload.code, payload.message, payload.detail);
  }
  return new GitTidyError("ipc_unknown", String(raw));
}
