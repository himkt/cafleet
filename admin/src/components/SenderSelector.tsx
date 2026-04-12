import { useMemo } from "react";
import type { Agent } from "../types";

interface SenderSelectorProps {
  agents: Agent[];
  tenantId: string;
  onSelect: (agentId: string | null) => void;
}

function storageKey(tenantId: string): string {
  return `hikyaku.sender.${tenantId}`;
}

function resolveStored(tenantId: string, activeAgents: Agent[]): string | null {
  const stored = localStorage.getItem(storageKey(tenantId));
  if (stored && activeAgents.some((a) => a.agent_id === stored)) {
    return stored;
  }
  return null;
}

export default function SenderSelector({
  agents,
  tenantId,
  onSelect,
}: SenderSelectorProps) {
  const activeAgents = agents.filter((a) => a.status === "active");

  const selectedId = useMemo(
    () => resolveStored(tenantId, activeAgents),
    [tenantId, activeAgents],
  );

  const handleChange = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const value = e.target.value || null;
    if (value) {
      localStorage.setItem(storageKey(tenantId), value);
    } else {
      localStorage.removeItem(storageKey(tenantId));
    }
    onSelect(value);
  };

  return (
    <select
      value={selectedId ?? ""}
      onChange={handleChange}
      className="border border-gray-300 rounded-md px-2 py-1 text-sm"
    >
      <option value="">Send as...</option>
      {activeAgents.map((a) => (
        <option key={a.agent_id} value={a.agent_id}>
          {a.name}
        </option>
      ))}
    </select>
  );
}
