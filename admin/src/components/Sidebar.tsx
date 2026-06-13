import type { Agent } from "../types";
import AgentAvatar from "./AgentAvatar";

interface SidebarProps {
  agents: Agent[];
  onSelectAgent: (agentId: number) => void;
}

function byRegisteredAt(a: Agent, b: Agent): number {
  return a.registered_at.localeCompare(b.registered_at);
}

function AgentRow({
  agent,
  dimmed,
  onSelect,
}: {
  agent: Agent;
  dimmed: boolean;
  onSelect: (agentId: number) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onSelect(agent.agent_id)}
      title={agent.name}
      className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-accent ${
        dimmed ? "opacity-50" : ""
      }`}
    >
      <AgentAvatar agent={agent} size="sm" />
      <span className="min-w-0 flex-1 truncate">{agent.name}</span>
      {agent.kind === "builtin-administrator" && (
        <span className="shrink-0 rounded bg-accent-soft px-1.5 py-0.5 text-[10px] font-medium text-accent">
          Admin
        </span>
      )}
      {agent.monitor !== null && (
        <span
          title={
            agent.monitor.enabled
              ? `Monitoring every ${agent.monitor.interval_seconds}s`
              : "Monitoring disabled"
          }
          className={`shrink-0 rounded px-1 py-0.5 font-mono text-[10px] ${
            agent.monitor.enabled
              ? "bg-surface-hover text-text-muted"
              : "bg-surface-hover text-text-faint line-through"
          }`}
        >
          {agent.monitor.enabled ? `${agent.monitor.interval_seconds}s` : "off"}
        </span>
      )}
      <span
        aria-hidden="true"
        className={`size-2 shrink-0 rounded-full ${
          agent.status === "active" ? "bg-success" : "bg-text-faint"
        }`}
      />
    </button>
  );
}

interface AgentGroupProps {
  heading: string;
  agents: Agent[];
  dimmed: boolean;
  onSelectAgent: (agentId: number) => void;
}

function AgentGroup({ heading, agents, dimmed, onSelectAgent }: AgentGroupProps) {
  if (agents.length === 0) return null;
  return (
    <div className="px-2 pt-3 pb-1">
      <h3 className="mb-1 px-2 text-[10px] font-semibold uppercase tracking-wider text-text-faint">
        {heading}
      </h3>
      {agents.map((a) => (
        <AgentRow
          key={a.agent_id}
          agent={a}
          dimmed={dimmed}
          onSelect={onSelectAgent}
        />
      ))}
    </div>
  );
}

export default function Sidebar({ agents, onSelectAgent }: SidebarProps) {
  const active = agents
    .filter((a) => a.status === "active")
    .sort(byRegisteredAt);
  const deregistered = agents
    .filter((a) => a.status === "deregistered")
    .sort(byRegisteredAt);

  return (
    <aside className="w-60 shrink-0 overflow-y-auto border-r border-border">
      <AgentGroup
        heading="Active"
        agents={active}
        dimmed={false}
        onSelectAgent={onSelectAgent}
      />
      <AgentGroup
        heading="Deregistered"
        agents={deregistered}
        dimmed={true}
        onSelectAgent={onSelectAgent}
      />
      {agents.length === 0 && (
        <p className="p-3 text-xs text-text-faint">
          No agents registered in this fleet. Use the{" "}
          <code className="font-mono text-text-muted">cafleet agent register</code>{" "}
          CLI to add one.
        </p>
      )}
    </aside>
  );
}
