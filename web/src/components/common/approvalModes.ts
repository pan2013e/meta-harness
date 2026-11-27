import { LockKeyhole, ShieldCheck, Unlock, type LucideIcon } from "lucide-react";
import type { PermissionPolicy } from "../../types/api";

export const APPROVAL_MODES = [
  {
    value: "approve_for_me",
    label: "Approve for me",
    description: "Use automatic review where the harness supports it",
    Icon: ShieldCheck,
    color: "text-brand-ink",
  },
  {
    value: "require_approval",
    label: "Require approval",
    description: "Ask before editing outside the workspace or accessing the Internet",
    Icon: LockKeyhole,
    color: "text-warn",
  },
  {
    value: "full_access",
    label: "Full access",
    description: "Allow file edits and Internet access without asking",
    Icon: Unlock,
    color: "text-neg",
  },
] as const satisfies ReadonlyArray<{
  value: PermissionPolicy;
  label: string;
  description: string;
  Icon: LucideIcon;
  color: string;
}>;
