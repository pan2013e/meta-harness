import { GitBranch, LockKeyhole, Zap, type LucideIcon } from "lucide-react";
import type { PermissionPolicy } from "../../types/api";

export const APPROVAL_MODES = [
  {
    value: "worktree_smart",
    label: "Worktree",
    description: "Allow worktree edits and prompt for shell commands",
    Icon: GitBranch,
    color: "text-brand-ink",
  },
  {
    value: "strict",
    label: "Strict",
    description: "Prompt for edits and shell commands",
    Icon: LockKeyhole,
    color: "text-warn",
  },
  {
    value: "autonomous_sandbox",
    label: "Auto",
    description: "Allow worktree edits and eligible local commands",
    Icon: Zap,
    color: "text-pos",
  },
] as const satisfies ReadonlyArray<{
  value: PermissionPolicy;
  label: string;
  description: string;
  Icon: LucideIcon;
  color: string;
}>;
