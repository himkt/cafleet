import { useState, useRef, useEffect, useLayoutEffect, useMemo } from "react";
import { LoaderCircle, Send, Users } from "lucide-react";
import type { Agent } from "../types";
import { sendMessage } from "../api";
import AgentAvatar from "./AgentAvatar";

interface MessageInputProps {
  senderId: number | null;
  agents: Agent[];
  onSent: () => void;
}

function slugify(name: string): string {
  return name.replace(/[^A-Za-z0-9]+/g, "-").toLowerCase();
}

type ParseResult = { to: number | "*"; body: string; error: string | null };

function parseError(error: string): ParseResult {
  return { to: 0, body: "", error };
}

function parseInput(raw: string, recipients: Agent[]): ParseResult {
  const mentionRe = /^@([A-Za-z0-9_-]+)(?:\s|$)/;
  const mentions: string[] = [];
  let rest = raw.trimStart();

  while (true) {
    const match = rest.match(mentionRe);
    if (!match) break;
    mentions.push(match[1]);
    rest = rest.slice(match[0].length).trimStart();
  }

  if (mentions.length === 0) {
    return parseError("Start the message with @<agent> or @all");
  }

  const hasAll = mentions.some((m) => m.toLowerCase() === "all");
  const nonAll = mentions.filter((m) => m.toLowerCase() !== "all");

  if (hasAll && nonAll.length > 0) {
    return parseError("@all cannot be combined with other mentions");
  }

  const body = rest.trim();

  if (hasAll) {
    if (!body) return parseError("Message body is empty");
    return { to: "*", body, error: null };
  }

  if (nonAll.length > 1) {
    return parseError(
      "Multi-recipient unicast is not supported; use @all for broadcast",
    );
  }

  const slug = nonAll[0];
  const matched = recipients.filter(
    (a) => slugify(a.name) === slug.toLowerCase(),
  );

  if (matched.length === 0) {
    return parseError(`No active agent named '@${slug}'`);
  }
  if (matched.length > 1) {
    return parseError(`Ambiguous mention '@${slug}'`);
  }
  if (!body) return parseError("Message body is empty");

  return { to: matched[0].agent_id, body, error: null };
}

type MentionCandidate =
  | { kind: "virtual"; label: string }
  | { kind: "agent"; agent: Agent };

interface MentionState {
  query: string;
  anchor: number;
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

  const disabled = !senderId || activeAgents.length === 0;

  const candidates: MentionCandidate[] = useMemo(() => {
    if (mention === null) return [];
    const q = mention.query.toLowerCase();
    const list: MentionCandidate[] = [];
    if ("all".startsWith(q)) {
      list.push({ kind: "virtual", label: "all" });
    }
    const matchedAgents = userAgents
      .filter((a) => slugify(a.name).startsWith(q))
      .sort((a, b) => a.name.localeCompare(b.name));
    list.push(...matchedAgents.map((agent) => ({ kind: "agent" as const, agent })));
    return list.slice(0, 6);
  }, [mention, userAgents]);

  const popoverOpen = candidates.length > 0;

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

  const insertCandidate = (candidate: MentionCandidate) => {
    const ta = textareaRef.current;
    const value = ta?.value ?? input;
    const cursor = ta?.selectionStart ?? value.length;
    const currentMention = detectMention(value, cursor);
    if (currentMention === null) {
      setMention(null);
      return;
    }
    const slug = candidate.kind === "virtual" ? candidate.label : slugify(candidate.agent.name);
    const replacement = `@${slug} `;
    // Scan right from the `@` to find the actual end of the mention token
    // so a caret inside the token (e.g. `@al|x`) still rewrites the full
    // `@alx`, not just `@al`.
    let tokenEnd = currentMention.anchor + 1;
    while (tokenEnd < value.length && /[A-Za-z0-9_-]/.test(value[tokenEnd])) {
      tokenEnd++;
    }
    const replaceEnd = Math.max(cursor, tokenEnd);
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
      if (e.shiftKey) return;
      if (popoverOpen) {
        e.preventDefault();
        const candidate = candidates[selectedIndex];
        if (candidate) insertCandidate(candidate);
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
        const candidate = candidates[selectedIndex];
        if (candidate) insertCandidate(candidate);
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
      className="relative shrink-0 border-t border-border bg-surface-raised p-3"
    >
      {popoverOpen && (
        <div className="absolute bottom-full left-3 right-3 z-10 mb-1 overflow-hidden rounded-lg border border-border bg-surface-raised shadow-lg">
          {candidates.map((candidate, idx) => {
            const key =
              candidate.kind === "virtual"
                ? `virtual:${candidate.label}`
                : `agent:${candidate.agent.agent_id}`;
            const slug =
              candidate.kind === "virtual" ? candidate.label : slugify(candidate.agent.name);
            const display = candidate.kind === "virtual" ? candidate.label : candidate.agent.name;
            const selected = idx === selectedIndex;
            return (
              <button
                type="button"
                key={key}
                onMouseDown={(ev) => {
                  ev.preventDefault();
                  insertCandidate(candidate);
                  textareaRef.current?.focus();
                }}
                onMouseEnter={() => setSelectedIndex(idx)}
                className={`flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent ${
                  selected ? "bg-accent-soft" : ""
                }`}
              >
                {candidate.kind === "agent" ? (
                  <AgentAvatar agent={candidate.agent} size="sm" />
                ) : (
                  <span
                    aria-hidden="true"
                    className="inline-flex size-6 shrink-0 items-center justify-center rounded-full bg-accent-soft text-accent"
                  >
                    <Users size={12} />
                  </span>
                )}
                <span className="shrink-0 font-mono text-xs text-text-muted">
                  @{slug}
                </span>
                <span className="truncate">{display}</span>
              </button>
            );
          })}
        </div>
      )}
      <div className="flex items-end gap-2 rounded-xl border border-border bg-surface px-3 py-2 focus-within:border-accent focus-within:ring-2 focus-within:ring-accent/30">
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
          className="max-h-36 flex-1 resize-none overflow-y-auto whitespace-pre-wrap bg-transparent text-sm outline-none placeholder:text-text-faint disabled:opacity-50"
        />
        <button
          type="submit"
          disabled={disabled || sending || !input.trim()}
          aria-label="Send"
          title="Send"
          className="rounded-lg bg-accent p-2 text-accent-fg hover:bg-accent-hover focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-50"
        >
          {sending ? (
            <LoaderCircle
              size={16}
              className="motion-safe:animate-spin"
              aria-hidden="true"
            />
          ) : (
            <Send size={16} aria-hidden="true" />
          )}
        </button>
      </div>
      <p className="mt-1.5 text-[11px] text-text-faint">
        <kbd className="rounded border border-border bg-surface-hover px-1 font-mono">
          Enter
        </kbd>{" "}
        to send ·{" "}
        <kbd className="rounded border border-border bg-surface-hover px-1 font-mono">
          Shift+Enter
        </kbd>{" "}
        for newline
      </p>
      {error && <p className="mt-1 text-xs text-danger">{error}</p>}
    </form>
  );
}
