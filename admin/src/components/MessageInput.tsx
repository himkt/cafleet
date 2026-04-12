import { useState } from "react";
import type { Agent } from "../types";
import { sendMessage } from "../api";

interface MessageInputProps {
  senderId: string | null;
  agents: Agent[];
  onSent: () => void;
}

function slugify(name: string): string {
  return name.replace(/[^A-Za-z0-9]+/g, "-").toLowerCase();
}

function parseInput(
  raw: string,
  activeAgents: Agent[],
): { to: string; body: string; error: string | null } {
  const trimmed = raw.trimStart();
  const mentionRe = /^@([A-Za-z0-9_-]+)(?:\s|$)/;
  const mentions: string[] = [];
  let rest = trimmed;

  while (true) {
    const match = rest.match(mentionRe);
    if (!match) break;
    mentions.push(match[1]);
    rest = rest.slice(match[0].length).trimStart();
  }

  if (mentions.length === 0) {
    return { to: "", body: "", error: "Start the message with @<agent> or @all" };
  }

  const hasAll = mentions.some((m) => m.toLowerCase() === "all");
  const nonAll = mentions.filter((m) => m.toLowerCase() !== "all");

  if (hasAll && nonAll.length > 0) {
    return { to: "", body: "", error: "@all cannot be combined with other mentions" };
  }

  if (hasAll) {
    const body = rest.trim();
    if (!body) return { to: "", body: "", error: "Message body is empty" };
    return { to: "*", body, error: null };
  }

  if (nonAll.length > 1) {
    return {
      to: "",
      body: "",
      error:
        "Multi-recipient unicast not supported in first cut; use @all for broadcast",
    };
  }

  const slug = nonAll[0];
  const matched = activeAgents.filter(
    (a) => slugify(a.name) === slug.toLowerCase(),
  );

  if (matched.length === 0) {
    return { to: "", body: "", error: `No active agent named '@${slug}'` };
  }

  if (matched.length > 1) {
    return { to: "", body: "", error: `Ambiguous mention '@${slug}'` };
  }

  const body = rest.trim();
  if (!body) return { to: "", body: "", error: "Message body is empty" };

  return { to: matched[0].agent_id, body, error: null };
}

export default function MessageInput({
  senderId,
  agents,
  onSent,
}: MessageInputProps) {
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const disabled = !senderId || agents.filter((a) => a.status === "active").length === 0;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (disabled || !senderId) return;

    const activeAgents = agents.filter((a) => a.status === "active");
    const parsed = parseInput(input, activeAgents);

    if (parsed.error) {
      setError(parsed.error);
      return;
    }

    setError(null);
    setSending(true);
    try {
      await sendMessage(senderId, parsed.to, parsed.body);
      setInput("");
      onSent();
    } catch (err) {
      setError(err instanceof Error ? err.message : "Send failed");
    } finally {
      setSending(false);
    }
  };

  return (
    <form
      onSubmit={handleSubmit}
      className="border-t border-gray-200 p-3 bg-gray-50"
    >
      <div className="flex gap-2">
        <input
          type="text"
          value={input}
          onChange={(e) => {
            setInput(e.target.value);
            setError(null);
          }}
          placeholder={
            disabled
              ? "Select a sender to start messaging"
              : "@agent or @all message..."
          }
          disabled={disabled || sending}
          className="flex-1 border border-gray-300 rounded-md px-3 py-1.5 text-sm disabled:opacity-50"
        />
        <button
          type="submit"
          disabled={disabled || sending || !input.trim()}
          className="bg-blue-600 text-white px-4 py-1.5 rounded-md text-sm hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {sending ? "..." : "Send"}
        </button>
      </div>
      {error && <p className="text-xs text-red-600 mt-1">{error}</p>}
    </form>
  );
}
