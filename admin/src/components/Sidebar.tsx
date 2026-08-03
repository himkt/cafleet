import type { Member } from "../types";
import MemberAvatar from "./MemberAvatar";

interface SidebarProps {
  members: Member[];
  onSelectMember: (memberId: number) => void;
}

function byRegisteredAt(a: Member, b: Member): number {
  return a.registered_at.localeCompare(b.registered_at);
}

function MemberRow({
  member,
  dimmed,
  onSelect,
}: {
  member: Member;
  dimmed: boolean;
  onSelect: (memberId: number) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onSelect(member.member_id)}
      title={member.name}
      className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-accent ${
        dimmed ? "opacity-50" : ""
      }`}
    >
      <MemberAvatar member={member} size="sm" />
      <span className="min-w-0 flex-1 truncate">{member.name}</span>
      <span
        aria-hidden="true"
        className={`size-2 shrink-0 rounded-full ${
          member.status === "active" ? "bg-success" : "bg-text-faint"
        }`}
      />
    </button>
  );
}

interface MemberGroupProps {
  heading: string;
  members: Member[];
  dimmed: boolean;
  onSelectMember: (memberId: number) => void;
}

function MemberGroup({
  heading,
  members,
  dimmed,
  onSelectMember,
}: MemberGroupProps) {
  if (members.length === 0) return null;
  return (
    <div className="px-2 pt-3 pb-1">
      <h3 className="mb-1 px-2 text-[10px] font-semibold uppercase tracking-wider text-text-faint">
        {heading}
      </h3>
      {members.map((m) => (
        <MemberRow
          key={m.member_id}
          member={m}
          dimmed={dimmed}
          onSelect={onSelectMember}
        />
      ))}
    </div>
  );
}

export default function Sidebar({ members, onSelectMember }: SidebarProps) {
  const active = members
    .filter((m) => m.status === "active")
    .sort(byRegisteredAt);
  const deregistered = members
    .filter((m) => m.status === "deregistered")
    .sort(byRegisteredAt);

  return (
    <aside className="w-60 shrink-0 overflow-y-auto border-r border-border">
      <MemberGroup
        heading="Active"
        members={active}
        dimmed={false}
        onSelectMember={onSelectMember}
      />
      <MemberGroup
        heading="Deregistered"
        members={deregistered}
        dimmed={true}
        onSelectMember={onSelectMember}
      />
      {members.length === 0 && (
        <p className="p-3 text-xs text-text-faint">
          No members registered in this fleet. Use the{" "}
          <code className="font-mono text-text-muted">cafleet member create</code>{" "}
          CLI to add one.
        </p>
      )}
    </aside>
  );
}
