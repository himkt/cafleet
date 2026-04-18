import type { Agent } from "../types";

interface SidebarProps {
  agents: Agent[];
}

function byRegisteredAt(a: Agent, b: Agent): number {
  return a.registered_at.localeCompare(b.registered_at);
}

interface AgentGroupProps {
  heading: string;
  agents: Agent[];
  rowClassName: string;
}

function AgentGroup({ heading, agents, rowClassName }: AgentGroupProps) {
  if (agents.length === 0) return null;
  return (
    <div className="px-3 pt-3 pb-1">
      <h3 className="text-[10px] font-semibold uppercase tracking-wider text-gray-400 mb-1">
        {heading}
      </h3>
      {agents.map((a) => (
        <div key={a.agent_id} className={rowClassName} title={a.name}>
          {a.name}
        </div>
      ))}
    </div>
  );
}

export default function Sidebar({ agents }: SidebarProps) {
  const active = agents
    .filter((a) => a.status === "active")
    .sort(byRegisteredAt);
  const deregistered = agents
    .filter((a) => a.status === "deregistered")
    .sort(byRegisteredAt);

  return (
    <aside className="w-48 shrink-0 border-r border-gray-200 bg-gray-50 overflow-y-auto">
      <AgentGroup
        heading="Active"
        agents={active}
        rowClassName="text-sm text-gray-800 py-1 px-1 truncate"
      />
      <AgentGroup
        heading="Deregistered"
        agents={deregistered}
        rowClassName="text-sm text-gray-400 py-1 px-1 truncate opacity-50 pointer-events-none"
      />
      {agents.length === 0 && (
        <p className="text-xs text-gray-400 p-3">
          No agents registered in this session. Use the{" "}
          <code className="text-gray-500">cafleet register</code> CLI to add
          one.
        </p>
      )}
    </aside>
  );
}
