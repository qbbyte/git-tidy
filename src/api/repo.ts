import { call } from "./client";

export interface RepoInfo {
  workTree: string;
  gitDir: string;
  gitVersion: string;
  branch: string | null;
  headCommit: string | null;
  dirty: boolean;
}

export function probeRepo(path: string) {
  return call<RepoInfo>("repo_probe", { path });
}
