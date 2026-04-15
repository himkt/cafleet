import { useState, useRef, useEffect, useLayoutEffect } from "react";
import type { Agent } from "../types";
import { sendMessage } from "../api";

interface MessageInputProps {
  senderId: string | null;
  agents: Agent[];
  onSent: () => void;
  disabled?: boolean;
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

type MentionCandidate =
  | { kind: "virtual"; label: string }
  | { kind: "agent"; agent: Agent };

interface MentionState {
  query: string;
  anchor: number; // index of the `@` character in the textarea value
}

function detectMention(text: string, cursor: number): MentionState | null {
  const before = text.slice(0, cursor);
  const atIdx = before.lastIndexOf("@");
  if (atIdx === -1) return null;
  const prevChar = atIdx === 0 ? "" : before[atIdx - 1];
  if (prevChar !== "" && !/\s/.test(prevChar)) return null;
  const substr = before.slice(atIdx);
  const m = substr.match(/^@([A-Za-z0-9_-]*)$/);
  if (!m) return null;
  return { query: m[1], anchor: atIdx };
}

export default function MessageInput({
  senderId,
  agents,
  onSent,
  disabled: disabledProp = false,
}: MessageInputProps) {
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [mention, setMention] = useState<MentionState | null>(null);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const pendingCursorRef = useRef<number | null>(null);
  const closePopoverTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );

  const clearClosePopoverTimeout = () => {
    if (closePopoverTimeoutRef.current !== null) {
      clearTimeout(closePopoverTimeoutRef.current);
      closePopoverTimeoutRef.current = null;
    }
  };

  useEffect(() => {
    return () => clearClosePopoverTimeout();
  }, []);

  const activeAgents = agents.filter((a) => a.status === "active");
  const userAgents = activeAgents.filter(
    (a) => a.kind !== "builtin-administrator",
  );

  const disabled = disabledProp || !senderId || activeAgents.length === 0;

  const candidates: MentionCandidate[] = (() => {
    if (mention === null) return [];
    const q = mention.query.toLowerCase();
    const list: MentionCandidate[] = [];
    if ("all".startsWith(q)) {
      list.push({ kind: "virtual", label: "all" });
    }
    const matchedAgents = userAgents
      .filter((a) => slugify(a.name).startsWith(q))
      .sort((a, b) => a.name.localeCompare(b.name));
    for (const a of matchedAgents) {
      list.push({ kind: "agent", agent: a });
    }
    return list.slice(0, 6);
  })();

  const popoverOpen = mention !== null && candidates.length > 0;

  useEffect(() => {
    if (!popoverOpen) {
      setSelectedIndex(0);
      return;
    }
    setSelectedIndex((prev) => {
      if (prev < 0) return 0;
      if (prev >= candidates.length) return Math.max(0, candidates.length - 1);
      return prev;
    });
  }, [popoverOpen, candidates.length]);

  useLayoutEffect(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${ta.scrollHeight}px`;
    if (pendingCursorRef.current !== null) {
      const pos = pendingCursorRef.current;
      pendingCursorRef.current = null;
      ta.selectionStart = pos;
      ta.selectionEnd = pos;
      ta.focus();
    }
  }, [input]);

  useEffect(() => {
    const ta = textareaRef.current;
    if (!ta) return;
    const syncMentionFromSelection = () => {
      const value = ta.value;
      const cursor = ta.selectionStart ?? value.length;
      setMention(detectMention(value, cursor));
    };
    ta.addEventListener("keyup", syncMentionFromSelection);
    ta.addEventListener("mouseup", syncMentionFromSelection);
    ta.addEventListener("select", syncMentionFromSelection);
    return () => {
      ta.removeEventListener("keyup", syncMentionFromSelection);
      ta.removeEventListener("mouseup", syncMentionFromSelection);
      ta.removeEventListener("select", syncMentionFromSelection);
    };
  }, []);

  const handleChange = (e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const value = e.target.value;
    const cursor = e.target.selectionStart ?? value.length;
    setInput(value);
    setError(null);
    setMention(detectMention(value, cursor));
  };

  const insertCandidate = (c: MentionCandidate) => {
    const ta = textareaRef.current;
    const value = ta?.value ?? input;
    const cursor = ta?.selectionStart ?? value.length;
    const currentMention = detectMention(value, cursor);
    if (currentMention === null) {
      setMention(null);
      return;
    }
    const slug = c.kind === "virtual" ? c.label : slugify(c.agent.name);
    const replacement = `@${slug} `;
    const mentionEnd =
      currentMention.anchor + currentMention.query.length + 1;
    const replaceEnd = Math.max(cursor, mentionEnd);
    const newValue =
      value.slice(0, currentMention.anchor) +
      replacement +
      value.slice(replaceEnd);
    const newCursor = currentMention.anchor + replacement.length;
    pendingCursorRef.current = newCursor;
    setInput(newValue);
    setMention(null);
  };

  const closePopover = () => setMention(null);

  const submitForm = async () => {
    if (disabled || !senderId) return;
    const parsed = parseInput(input, userAgents);
    if (parsed.error) {
      setError(parsed.error);
      return;
    }
    setError(null);
    setSending(true);
    try {
      await sendMessage(senderId, parsed.to, parsed.body);
      setInput("");
      setMention(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Send failed");
      return;
    } finally {
      setSending(false);
    }
    onSent();
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const composing = e.nativeEvent.isComposing;

    if (e.key === "ArrowDown" && popoverOpen && !composing) {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, candidates.length - 1));
      return;
    }
    if (e.key === "ArrowUp" && popoverOpen && !composing) {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
      return;
    }
    if (e.key === "Enter") {
      if (composing) return;
      if (e.shiftKey) return; // default textarea newline
      if (popoverOpen) {
        e.preventDefault();
        insertCandidate(candidates[selectedIndex]);
        return;
      }
      e.preventDefault();
      void submitForm();
      return;
    }
    if (e.key === "Tab") {
      if (composing) return;
      if (popoverOpen) {
        e.preventDefault();
        insertCandidate(candidates[selectedIndex]);
      }
      return;
    }
    if (e.key === "Escape" && popoverOpen) {
      e.preventDefault();
      closePopover();
    }
  };

  const handleBlur = () => {
    // Delay to let row onMouseDown fire first when the user clicks a candidate.
    clearClosePopoverTimeout();
    closePopoverTimeoutRef.current = setTimeout(() => {
      closePopoverTimeoutRef.current = null;
      closePopover();
    }, 100);
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    void submitForm();
  };

  return (
    <form
      onSubmit={handleSubmit}
      className="border-t border-gray-200 p-3 bg-gray-50 relative"
    >
      {popoverOpen && (
        <div className="absolute bottom-full left-3 right-3 mb-1 bg-white border border-gray-200 rounded-md shadow-lg overflow-hidden z-10">
          {candidates.map((c, idx) => {
            const key =
              c.kind === "virtual"
                ? `virtual:${c.label}`
                : `agent:${c.agent.agent_id}`;
            const slug =
              c.kind === "virtual" ? c.label : slugify(c.agent.name);
            const display = c.kind === "virtual" ? c.label : c.agent.name;
            const selected = idx === selectedIndex;
            return (
              <button
                type="button"
                key={key}
                onMouseDown={(ev) => {
                  ev.preventDefault();
                  insertCandidate(c);
                  textareaRef.current?.focus();
                }}
                onMouseEnter={() => setSelectedIndex(idx)}
                className={`w-full text-left px-3 py-1.5 text-sm ${
                  selected ? "bg-blue-50" : ""
                }`}
              >
                <span className="font-mono text-gray-500 mr-2">@{slug}</span>
                <span className="text-gray-900">{display}</span>
              </button>
            );
          })}
        </div>
      )}
      <div className="flex gap-2 items-end">
        <textarea
          ref={textareaRef}
          rows={1}
          value={input}
          onChange={handleChange}
          onKeyDown={handleKeyDown}
          onBlur={handleBlur}
          placeholder={
            disabled
              ? "Administrator unavailable — messaging disabled"
              : "@agent or @all message..."
          }
          disabled={disabled || sending}
          className="flex-1 border border-gray-300 rounded-md px-3 py-1.5 text-sm disabled:opacity-50 resize-none whitespace-pre-wrap max-h-36 overflow-y-auto"
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
