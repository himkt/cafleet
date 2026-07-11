import type { Member } from "../types";

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

interface MemberAvatarProps {
  member: Pick<Member, "member_id" | "name" | "kind">;
  size?: AvatarSize;
}

export default function MemberAvatar({ member, size = "md" }: MemberAvatarProps) {
  const colorClass = `${PALETTE[member.member_id % PALETTE.length]} text-white`;
  return (
    <span
      aria-hidden="true"
      className={`inline-flex shrink-0 select-none items-center justify-center rounded-full font-medium ${SIZE_CLASSES[size]} ${colorClass}`}
    >
      {member.name.slice(0, 2)}
    </span>
  );
}
