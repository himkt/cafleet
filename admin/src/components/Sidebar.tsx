import type { Agent } from "../types";

interface SidebarProps {
  agents: Agent[];
}

function byRegisteredAt(a: Agent, b: Agent): number {
  return a.registered_at.localeCompare(b.registered_at);
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
      {active.length > 0 && (
        <div className="px-3 pt-3 pb-1">
          <h3 className="text-[10px] font-semibold uppercase tracking-wider text-gray-400 mb-1">
            Active
          </h3>
          {active.map((a) => (
            <div
              key={a.agent_id}
              className="text-sm text-gray-800 py-1 px-1 truncate"
              title={a.name}
            >
              {a.name}
            </div>
          ))}
        </div>
      )}
      {deregistered.length > 0 && (
        <div className="px-3 pt-3 pb-1">
          <h3 className="text-[10px] font-semibold uppercase tracking-wider text-gray-400 mb-1">
            Deregistered
          </h3>
          {deregistered.map((a) => (
            <div
              key={a.agent_id}
              className="text-sm text-gray-400 py-1 px-1 truncate opacity-50 pointer-events-none"
              title={a.name}
            >
              {a.name}
            </div>
          ))}
        </div>
      )}
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
