import { Zap } from "lucide-react";
import type { Agent } from "../types";

const PALETTE = [
  "bg-avatar-1",
  "bg-avatar-2",
  "bg-avatar-3",
  "bg-avatar-4",
  "bg-avatar-5",
  "bg-avatar-6",
  "bg-avatar-7",
  "bg-avatar-8",
  "bg-avatar-9",
  "bg-avatar-10",
  "bg-avatar-11",
  "bg-avatar-12",
] as const;

type AvatarSize = "sm" | "md" | "lg";

const SIZE_CLASSES: Record<AvatarSize, string> = {
  sm: "size-6 text-[10px]",
  md: "size-8 text-xs",
  lg: "size-12 text-base",
};

const ICON_SIZES: Record<AvatarSize, number> = {
  sm: 12,
  md: 14,
  lg: 20,
};

interface AgentAvatarProps {
  agent: Pick<Agent, "agent_id" | "name" | "kind">;
  size?: AvatarSize;
}

export default function AgentAvatar({ agent, size = "md" }: AgentAvatarProps) {
  const isAdministrator = agent.kind === "builtin-administrator";
  const colorClass = isAdministrator
    ? "bg-accent text-accent-fg"
    : `${PALETTE[agent.agent_id % PALETTE.length]} text-white`;
  return (
    <span
      aria-hidden="true"
      className={`inline-flex shrink-0 select-none items-center justify-center rounded-full font-medium ${SIZE_CLASSES[size]} ${colorClass}`}
    >
      {isAdministrator ? (
        <Zap size={ICON_SIZES[size]} fill="currentColor" />
      ) : (
        agent.name.slice(0, 2)
      )}
    </span>
  );
}
