import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent"
import { Type } from "typebox"
import { execFile, spawn, type ChildProcess } from "node:child_process"
import { accessSync, constants, readFileSync, statSync } from "node:fs"
import { homedir } from "node:os"
import { basename, isAbsolute, join, relative, resolve, sep } from "node:path"
import { createInterface } from "node:readline"
import { fileURLToPath } from "node:url"

const RECORD_PREFIX = "[cafleet listen record] "
const CLOSED_PREFIX = "[cafleet listen closed] "
const GOAL_PREFIX = "[cafleet goal] "
const LISTEN_TYPE = "cafleet-listen"
const GOAL_TYPE = "cafleet-goal"
const GOAL_NOTICE_TYPE = "cafleet-goal-notice"
const MEMBER_REFUSAL = "cafleet tools run only in a Director session"
const SETUP_FIX = "run 'cafleet setup --coding-agent pi'"
const RECEIVER_STOPPED = Symbol.for("cafleet.pi.receiver-stopped")
const IDLE_LIMIT = 3
const CONTINUATION_LIMIT = 50
const VERIFY_TIMEOUT_MS = 10_000
const RESUME_HINT = "the operator resumes it with /cafleet-goal resume"

const GATE_FLAG = "cafleet-gate"
const GATE_MODES = ["cafleet", "cafleet-ask"] as const
const GATE_TOOLS = "bash, read, grep, find, ls, edit and write"
const ROUTE_COMMAND = "route the command to the Director if it is required"
const ROUTE_CHANGE = "route the change to the Director if it is required"
const ROUTE_READ = "route the read to the Director if it is required"
const PROTECTED_DIRECTORIES = [".git", ".pi", ".agents", ".claude", ".codex", ".opencode"]
const ALLOW_ONCE = "Allow once"
const DENY = "Deny"

const MEMBER_ALLOWLIST = [
  "git add *", "git commit *", "git diff *", "git grep *", "git log *", "git ls-tree *",
  "git ls-files *", "git branch *", "git status", "grep *", "ls", "ls *", "stat *",
  "tree", "mise //:test", "mise //:test *", "mise //:lint", "mise //:format",
  "mise //:typecheck", "mise //:build", "wc *", "cafleet message send *",
  "cafleet message poll *", "cafleet message ack *", "cafleet message show *",
]

type GoalStatus = "active" | "paused" | "complete" | "cleared"

interface Goal {
  objective: string
  status: GoalStatus
  generation: number
  continuations: number
  idle_streak: number
}

interface DirectorInput {
  director_id: number
  database_url?: string
}

type GateMode = (typeof GATE_MODES)[number]
type Gate = { kind: "inert" } | { kind: "active"; mode: GateMode } | { kind: "failed"; reason: string }
type Block = { block: true; reason: string }

const result = (fields: Record<string, unknown>) => ({
  content: [{ type: "text" as const, text: JSON.stringify(fields) }],
  details: undefined,
  ...(fields.ok === false ? { isError: true } : {}),
})

function recordedExecutable(): string {
  const record = fileURLToPath(new URL("./cafleet.executable", import.meta.url))
  let target: string
  try {
    target = readFileSync(record, "utf8").trim()
    if (!target) throw new Error("the record is empty")
    if (!statSync(target).isFile()) throw new Error(`${target} is not a file`)
    accessSync(target, constants.X_OK)
  } catch (error) {
    throw new Error(`executable record ${record}: ${(error as Error).message}; ${SETUP_FIX}`)
  }
  return target
}

const databaseArgs = (databaseURL?: string) => (databaseURL ? ["--database-url", databaseURL] : [])

/** Resolves to undefined when `director_id` is a root Director, otherwise to the refusal text. */
function directorRefusal({ director_id, database_url }: DirectorInput): Promise<string | undefined> {
  let executable: string
  try {
    executable = recordedExecutable()
  } catch (error) {
    return Promise.resolve((error as Error).message)
  }
  const args = [...databaseArgs(database_url), "member", "show", String(director_id), "--json"]
  return new Promise((done) => {
    execFile(executable, args, { timeout: VERIFY_TIMEOUT_MS }, (error, stdout, stderr) => {
      if (!error) {
        try {
          if (JSON.parse(stdout).kind === "director") return done(undefined)
        } catch {}
      }
      done(`${MEMBER_REFUSAL}: ${String(stderr).trim() || (error?.message ?? "")}`)
    })
  })
}

const directorParameters = {
  director_id: Type.Integer({ description: "The Director's literal member ID." }),
  database_url: Type.Optional(
    Type.String({ description: "The fleet's database URL when the Director's DATABASE URL line names one." }),
  ),
}

/** A shell word after quote removal; `expands` marks a `$` outside single quotes or an unquoted glob or brace. */
interface Word {
  text: string
  expands: boolean
}

/** A `$` outside single quotes is plain parameter expansion only when a name follows it. */
const startsName = (char: string | undefined) => char !== undefined && /[A-Za-z_]/.test(char)

/** The words of `command`, which holds no control character, when it is one simple command, otherwise undefined. */
function simpleWords(command: string): Word[] | undefined {
  const words: Word[] = []
  let word: Word | undefined
  let quote: "'" | '"' | undefined
  const append = (text: string, expands = false) => {
    word ??= { text: "", expands: false }
    word.text += text
    word.expands ||= expands
  }
  for (let index = 0; index < command.length; index++) {
    const char = command[index]
    const next = command[index + 1]
    if (quote === "'") {
      if (char === "'") quote = undefined
      else append(char)
      continue
    }
    if (quote === '"') {
      if (char === '"') quote = undefined
      else if (char === "`" || (char === "$" && !startsName(next))) return undefined
      else if (char === "\\" && next !== undefined && '$`"\\'.includes(next)) {
        append(next)
        index++
      } else append(char, char === "$")
      continue
    }
    if (char === " " || char === "\t") {
      if (word) words.push(word)
      word = undefined
      continue
    }
    if (char === "\\") {
      if (next === undefined) return undefined
      append(next)
      index++
      continue
    }
    if (char === "'" || char === '"') {
      quote = char
      append("")
      continue
    }
    if (";&|<>()`".includes(char)) return undefined
    if (char === "$" && !startsName(next)) return undefined
    append(char, "$*?[{".includes(char))
  }
  if (quote !== undefined) return undefined
  if (word) words.push(word)
  return words
}

const isShortOptionWith = (text: string, letter: string) => text.startsWith("-") && !text.startsWith("--") && text.includes(letter)

function isRefusedGitWord(text: string): boolean {
  if (isShortOptionWith(text, "O")) return true
  if (!text.startsWith("--")) return false
  const name = text.slice(2).split("=")[0]
  return name === "output" || (name.length >= 2 && "open-files-in-pager".startsWith(name))
}

/**
 * A mise task forwards its words to cargo, whose options can choose the command cargo runs or where it writes,
 * and a `:::` word or a task reference (`//…`, `:…`) makes mise run another task.
 */
const isRefusedMiseWord = (text: string) => text.startsWith("-") || text.startsWith(":") || text.startsWith("/")

/** The commands whose words must stay literal, each with its refused option words. */
const LITERAL_WORD_COMMANDS = new Map([
  ["git", isRefusedGitWord],
  ["mise", isRefusedMiseWord],
])

/**
 * True when a git or mise word expands, or a word after the git subcommand or mise task names a refused option or task.
 * The allowlist pins the first two words, so the mise task word `//:test` itself is checked only for expansion.
 */
function hasRefusedWord(words: Word[]): boolean {
  const refused = LITERAL_WORD_COMMANDS.get(words[0]?.text ?? "")
  return refused !== undefined && words.some((word, index) => word.expands || (index >= 2 && refused(word.text)))
}

const ALLOWLIST_RULES = MEMBER_ALLOWLIST.map(
  (pattern) =>
    new RegExp(`^${pattern.split("*").map((part) => part.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&")).join(".*")}$`, "s"),
)

function isAllowlisted(command: string): boolean {
  const trimmed = command.trim()
  const words = simpleWords(trimmed)
  return words !== undefined && !hasRefusedWord(words) && ALLOWLIST_RULES.some((rule) => rule.test(trimmed))
}

const DEFAULT_PATH_TOOLS = ["grep", "find", "ls"]
const UNICODE_SPACES = /[\u00A0\u2000-\u200A\u202F\u205F\u3000]/g

/** The tool's path text; without a non-empty `path`, grep, find and ls default to "." and the others have none. */
function toolPathText(toolName: string, raw: unknown): string | undefined {
  if (typeof raw === "string" && raw !== "") return raw
  return DEFAULT_PATH_TOOLS.includes(toolName) ? "." : undefined
}

/** pi 1.5.0 file tools resolve `path` through `resolveToCwd`, which maps these spaces and drops one leading `@` first. */
function normalizeToolPath(text: string): string {
  const spaced = text.replace(UNICODE_SPACES, " ")
  return spaced.startsWith("@") ? spaced.slice(1) : spaced
}

/** pi percent-decodes a `file://` path, which `path.resolve` would read as a literal relative path. */
const isFileURL = (normalized: string) => normalized.toLowerCase().startsWith("file:")

function resolveToolPath(cwd: string, normalized: string): string {
  if (normalized === "~") return homedir()
  if (normalized.startsWith("~/")) return join(homedir(), normalized.slice(2))
  return resolve(cwd, normalized)
}

function isEnvFile(path: string): boolean {
  const name = basename(path).toLowerCase()
  return name === ".env" || name.startsWith(".env.")
}

function fileToolBlock(toolName: string, input: Record<string, unknown>, cwd: string): Block | undefined {
  const changes = toolName === "edit" || toolName === "write"
  const route = changes ? ROUTE_CHANGE : ROUTE_READ
  const text = toolPathText(toolName, input.path)
  if (text === undefined) {
    return { block: true, reason: `the CAFleet member gate cannot read the path input of ${toolName}; ${route}` }
  }
  const normalized = normalizeToolPath(text)
  if (isFileURL(normalized)) {
    return { block: true, reason: `the CAFleet member gate refuses a file: URL path for ${toolName}; ${route}` }
  }
  const path = resolveToolPath(cwd, normalized)
  if (isEnvFile(path)) {
    return { block: true, reason: `the CAFleet member gate protects .env files from ${toolName}; ${route}` }
  }
  if (!changes) return undefined
  const inside = relative(cwd, path)
  if (inside.startsWith("..") || isAbsolute(inside)) {
    return {
      block: true,
      reason: `the CAFleet member gate allows edit and write only inside the working directory; ${ROUTE_CHANGE}`,
    }
  }
  if (inside.split(sep).some((component) => PROTECTED_DIRECTORIES.includes(component.toLowerCase()))) {
    return {
      block: true,
      reason: `the CAFleet member gate protects .git, .pi, .agents, .claude, .codex and .opencode from edit and write; ${ROUTE_CHANGE}`,
    }
  }
  return undefined
}

const CONTROL_CHARACTER = /[\x00-\x08\x0a-\x1f\x7f-\x9f]/

/** The block for a command that neither mode runs or shows in a form, otherwise undefined. */
function controlBlock(command: string): Block | undefined {
  if (!CONTROL_CHARACTER.test(command)) return undefined
  return { block: true, reason: `the CAFleet member gate refuses a command holding a control character; ${ROUTE_COMMAND}` }
}

const approvalTitle = (command: string) => `CAFleet member gate: allow this bash command?\n\n$ ${command}`

export default function (pi: ExtensionAPI) {
  pi.registerFlag(GATE_FLAG, {
    description: "CAFleet member gate mode (cafleet or cafleet-ask); set only by cafleet member create",
    type: "string",
  })

  let settleGate: (gate: Gate) => void = () => undefined
  const gate = new Promise<Gate>((done) => {
    settleGate = done
  })
  let approvals: Promise<unknown> = Promise.resolve()

  let goal: Goal | undefined
  let pendingContinuation = false
  let progressed = false

  let receiver: ChildProcess | undefined
  let receiverEnded = true
  let receiverStopped = false

  const notify = (ctx: ExtensionContext, text: string, level: "info" | "warning" | "error" = "info") => {
    if (ctx.hasUI) ctx.ui.notify(text, level)
  }

  function restoreGoal(ctx: ExtensionContext) {
    goal = undefined
    for (const entry of ctx.sessionManager.getBranch()) {
      if (entry.type === "custom" && entry.customType === GOAL_TYPE) goal = entry.data as Goal
    }
  }

  function storeGoal(next: Goal) {
    goal = next
    pi.appendEntry(GOAL_TYPE, next)
  }

  const continuationText = (objective: string) =>
    `${GOAL_PREFIX}Continue working toward the active goal: ${objective}. ` +
    "Call cafleet_goal_progress after each step and cafleet_goal_complete when it is achieved."

  function sendContinuation(current: Goal) {
    pendingContinuation = true
    pi.sendMessage(
      { customType: GOAL_TYPE, content: continuationText(current.objective), display: true, details: { generation: current.generation } },
      { triggerTurn: true, deliverAs: "followUp" },
    )
  }

  function pauseGoal(current: Goal, changes: Partial<Goal> = {}) {
    storeGoal({ ...current, ...changes, status: "paused", generation: current.generation + 1 })
  }

  function failGoal(ctx: ExtensionContext, cause: unknown) {
    console.error("[cafleet goal] handler failed:", cause)
    if (goal?.status !== "active") return
    try {
      pauseGoal(goal)
      notify(
        ctx,
        `${GOAL_PREFIX}paused: the goal cannot continue (${(cause as Error).message}); ${RESUME_HINT}`,
        "error",
      )
    } catch (pauseError) {
      console.error("[cafleet goal] pausing the goal failed:", pauseError)
    }
  }

  function receiverLive() {
    return receiver !== undefined && !receiverEnded
  }

  function sendListen(content: string) {
    pi.sendMessage({ customType: LISTEN_TYPE, content, display: true }, { triggerTurn: true, deliverAs: "followUp" })
  }

  function stopReceiver() {
    if (!receiverLive()) return false
    receiverStopped = true
    receiver!.kill("SIGTERM")
    return true
  }

  function startReceiver({ director_id, database_url }: DirectorInput): ChildProcess {
    const child = spawn(recordedExecutable(), [...databaseArgs(database_url), "message", "listen", String(director_id)], {
      stdio: ["ignore", "pipe", "pipe"],
    })
    receiver = child
    receiverEnded = false
    receiverStopped = false
    let sawClosed = false
    let noticeSent = false
    let lastStderr = ""
    const closedNotice = (detail: string) => {
      if (noticeSent || receiverStopped || sawClosed) return
      noticeSent = true
      try {
        sendListen(`${CLOSED_PREFIX}${detail}`)
      } catch (error) {
        console.error("[cafleet listen] closed notice failed:", error)
      }
    }
    const end = (detail: string) => {
      if (receiver === child) receiverEnded = true
      closedNotice(detail)
    }
    createInterface({ input: child.stdout! }).on("line", (line) => {
      if (!line || noticeSent) return
      sawClosed ||= line.startsWith('{"event":"closed"')
      try {
        sendListen(RECORD_PREFIX + line)
      } catch (error) {
        child.kill("SIGTERM")
        if (receiver === child) receiverEnded = true
        sawClosed = false
        closedNotice(`delivery failed: ${(error as Error).message}`)
      }
    })
    createInterface({ input: child.stderr! }).on("line", (line) => {
      if (line.trim()) lastStderr = line.trim()
    })
    child.on("error", (error: NodeJS.ErrnoException) => end(`exit ${error.code ?? "error"}: ${error.message}`))
    child.on("close", (code, signal) => end(`exit ${code ?? signal}: ${lastStderr}`))
    return child
  }

  function readGate(): Gate {
    const value = pi.getFlag(GATE_FLAG)
    if (value === undefined) return { kind: "inert" }
    if (typeof value === "string" && (GATE_MODES as readonly string[]).includes(value)) {
      return { kind: "active", mode: value as GateMode }
    }
    return { kind: "failed", reason: `the CAFleet member gate received an unknown --cafleet-gate value '${String(value)}'` }
  }

  pi.on("session_start", async (_event, ctx) => {
    let resolved: Gate = { kind: "failed", reason: "the CAFleet member gate could not read --cafleet-gate: unread" }
    try {
      resolved = readGate()
      if (resolved.kind === "active" && ctx.hasUI) ctx.ui.setStatus("cafleet-gate", `cafleet gate: ${resolved.mode}`)
    } catch (error) {
      resolved = {
        kind: "failed",
        reason: `the CAFleet member gate could not read --cafleet-gate: ${(error as Error).message}`,
      }
    } finally {
      settleGate(resolved)
    }

    try {
      restoreGoal(ctx)
    } catch (error) {
      console.error("[cafleet goal] restoring the goal failed:", error)
    }

    const stopped = (globalThis as Record<symbol, unknown>)[RECEIVER_STOPPED] as { reason: string } | undefined
    if (stopped) {
      delete (globalThis as Record<symbol, unknown>)[RECEIVER_STOPPED]
      sendListen(`${CLOSED_PREFIX}the extension runtime was replaced (${stopped.reason}); re-arm with cafleet_listen`)
    }
  })

  pi.on("session_tree", async (_event, ctx) => {
    try {
      restoreGoal(ctx)
    } catch (error) {
      console.error("[cafleet goal] restoring the goal failed:", error)
    }
  })

  pi.on("session_shutdown", async (event) => {
    if (stopReceiver() && event.reason !== "quit") {
      ;(globalThis as Record<symbol, unknown>)[RECEIVER_STOPPED] = { reason: event.reason }
    }
  })

  async function gateBash(command: string, mode: GateMode, ctx: ExtensionContext): Promise<Block | undefined> {
    const refused = controlBlock(command)
    if (refused) return refused
    if (isAllowlisted(command)) return undefined
    if (mode === "cafleet") {
      return {
        block: true,
        reason: `the CAFleet member gate allows only allowlisted simple commands; ${ROUTE_COMMAND}`,
      }
    }
    if (!ctx.hasUI) {
      return { block: true, reason: `the approval form needs the interactive pi TUI; ${ROUTE_COMMAND}` }
    }
    const choice = await ctx.ui.select(approvalTitle(command), [ALLOW_ONCE, DENY])
    if (choice === ALLOW_ONCE) return undefined
    if (choice === DENY) return { block: true, reason: "the operator denied this command" }
    return { block: true, reason: "the approval form was dismissed; the command did not run" }
  }

  pi.on("tool_call", async (event, ctx) => {
    const current = await gate
    if (current.kind === "inert") return undefined
    if (current.kind === "failed") return { block: true, reason: current.reason }
    const input = event.input as Record<string, unknown>
    switch (event.toolName) {
      case "bash": {
        const command = typeof input.command === "string" ? input.command : ""
        const decision = approvals.then(() => gateBash(command, current.mode, ctx))
        approvals = decision.catch(() => undefined)
        return decision
      }
      case "read":
      case "grep":
      case "find":
      case "ls":
      case "edit":
      case "write":
        return fileToolBlock(event.toolName, input, ctx.cwd)
      default:
        if (event.toolName.startsWith("cafleet_")) return { block: true, reason: MEMBER_REFUSAL }
        return { block: true, reason: `the CAFleet member gate allows only ${GATE_TOOLS}` }
    }
  })

  pi.registerTool({
    name: "cafleet_listen",
    label: "CAFleet listen",
    description:
      "Start CAFleet Director reception for this session: every broker record addressed to the Director " +
      "arrives as a message beginning '[cafleet listen record]'.",
    parameters: Type.Object(directorParameters),
    async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
      const input = params as DirectorInput
      const refusal = await directorRefusal(input)
      if (refusal !== undefined) return result({ ok: false, error: refusal })
      const sessionId = ctx.sessionManager.getSessionId()
      if (receiverLive()) return result({ ok: true, pid: receiver!.pid, sessionId, existing: true })
      try {
        const child = startReceiver(input)
        if (child.pid === undefined) {
          receiverStopped = true
          receiverEnded = true
          return result({ ok: false, error: "cafleet message listen did not start" })
        }
        return result({ ok: true, pid: child.pid, sessionId })
      } catch (error) {
        return result({ ok: false, error: (error as Error).message })
      }
    },
  })

  pi.registerTool({
    name: "cafleet_ask",
    label: "CAFleet ask",
    description:
      "Ask the operator one question with listed choices plus Other; an answered result records the decision.",
    parameters: Type.Object({
      ...directorParameters,
      question: Type.String({ description: "The question shown to the operator." }),
      choices: Type.Array(Type.String(), { description: "1 to 9 distinct non-empty choices, none named Other." }),
    }),
    executionMode: "sequential",
    async execute(_toolCallId, params, _signal, _onUpdate, ctx) {
      const input = params as DirectorInput & { question: string; choices: string[] }
      const refusal = await directorRefusal(input)
      if (refusal !== undefined) return result({ ok: false, error: refusal })
      if (!ctx.hasUI) return result({ ok: false, error: "cafleet_ask needs the interactive pi TUI" })
      const { question, choices } = input
      if (choices.length < 1 || choices.length > 9) {
        return result({ ok: false, error: "choices must hold 1 to 9 entries" })
      }
      if (choices.some((choice) => !choice.trim())) return result({ ok: false, error: "choices must be non-empty" })
      if (new Set(choices).size !== choices.length) return result({ ok: false, error: "choices must be distinct" })
      if (choices.includes("Other")) return result({ ok: false, error: "choices must not include Other" })
      const choice = await ctx.ui.select(question, [...choices, "Other"])
      if (choice === undefined) return result({ ok: true, status: "cancelled" })
      if (choice !== "Other") {
        return result({ ok: true, status: "answered", answer: choice, choice: choices.indexOf(choice) + 1 })
      }
      const text = await ctx.ui.input(question, "")
      if (!text?.trim()) return result({ ok: true, status: "cancelled" })
      return result({ ok: true, status: "answered", answer: text, choice: null })
    },
  })

  pi.registerTool({
    name: "cafleet_goal_status",
    label: "CAFleet goal status",
    description: "Report the CAFleet goal of this session.",
    parameters: Type.Object(directorParameters),
    async execute(_toolCallId, params) {
      const refusal = await directorRefusal(params as DirectorInput)
      if (refusal !== undefined) return result({ ok: false, error: refusal })
      return { content: [{ type: "text" as const, text: JSON.stringify(goal ?? null) }], details: undefined }
    },
  })

  pi.registerTool({
    name: "cafleet_goal_progress",
    label: "CAFleet goal progress",
    description: "Report a completed step toward the active CAFleet goal of this session.",
    parameters: Type.Object({ ...directorParameters, note: Type.String() }),
    async execute(_toolCallId, params) {
      const refusal = await directorRefusal(params as DirectorInput)
      if (refusal !== undefined) return result({ ok: false, error: refusal })
      if (!goal) return result({ ok: false, error: "no CAFleet goal is set for this session" })
      storeGoal({ ...goal, idle_streak: 0 })
      progressed = true
      return result({ ok: true })
    },
  })

  pi.registerTool({
    name: "cafleet_goal_complete",
    label: "CAFleet goal complete",
    description: "Mark the active CAFleet goal of this session as achieved.",
    parameters: Type.Object({ ...directorParameters, summary: Type.String() }),
    async execute(_toolCallId, params) {
      const refusal = await directorRefusal(params as DirectorInput)
      if (refusal !== undefined) return result({ ok: false, error: refusal })
      if (!goal) return result({ ok: false, error: "no CAFleet goal is set for this session" })
      storeGoal({ ...goal, status: "complete", generation: goal.generation + 1 })
      progressed = true
      return result({ ok: true })
    },
  })

  pi.registerCommand("cafleet-goal", {
    description: "Set a CAFleet goal (text), or report status / pause / resume / clear it.",
    handler: async (args, ctx) => {
      try {
        const argument = (args ?? "").trim()
        if (argument === "" || argument === "status") {
          notify(ctx, `${GOAL_PREFIX}status: ${JSON.stringify(goal ?? null)}`)
          return
        }
        if (["pause", "resume", "clear"].includes(argument)) {
          if (!goal || goal.status === "cleared") {
            notify(ctx, `${GOAL_PREFIX}no CAFleet goal is set for this session`, "error")
            return
          }
          if (argument === "pause") pauseGoal(goal)
          else if (argument === "clear") {
            storeGoal({ ...goal, objective: "", status: "cleared", generation: goal.generation + 1 })
          } else {
            storeGoal({ ...goal, status: "active", generation: goal.generation + 1, continuations: 0, idle_streak: 0 })
            sendContinuation(goal)
          }
          return
        }
        storeGoal({
          objective: argument,
          status: "active",
          generation: (goal?.generation ?? 0) + 1,
          continuations: 0,
          idle_streak: 0,
        })
        sendContinuation(goal!)
      } catch (error) {
        failGoal(ctx, error)
      }
    },
  })

  pi.on("agent_before_settle", async (event, ctx) => {
    const wasContinuation = pendingContinuation
    const didProgress = progressed
    pendingContinuation = false
    progressed = false
    if (goal?.status !== "active") return undefined
    try {
      const idle = wasContinuation && !didProgress ? goal.idle_streak + 1 : goal.idle_streak
      const pausedNotice = (text: string) => ({
        entries: [
          ...event.entries,
          { type: "custom_message" as const, customType: GOAL_NOTICE_TYPE, content: `${GOAL_PREFIX}paused: ${text}; ${RESUME_HINT}`, display: true },
        ],
        continue: false,
      })
      if (idle >= IDLE_LIMIT) {
        pauseGoal(goal, { idle_streak: idle })
        return pausedNotice(`no progress in ${IDLE_LIMIT} consecutive continuation turns`)
      }
      if (goal.continuations >= CONTINUATION_LIMIT) {
        pauseGoal(goal, { idle_streak: idle })
        return pausedNotice(`reached ${CONTINUATION_LIMIT} continuations`)
      }
      storeGoal({ ...goal, idle_streak: idle, continuations: goal.continuations + 1 })
      pendingContinuation = true
      return {
        entries: [
          ...event.entries,
          {
            type: "custom_message" as const,
            customType: GOAL_TYPE,
            content: continuationText(goal.objective),
            display: true,
            details: { generation: goal.generation },
          },
        ],
        continue: true,
      }
    } catch (error) {
      pendingContinuation = false
      failGoal(ctx, error)
      return undefined
    }
  })

  pi.on("agent_settled", async (event, ctx) => {
    if (!event.aborted) return
    pendingContinuation = false
    progressed = false
    try {
      if (goal?.status !== "active") return
      pauseGoal(goal)
      notify(ctx, `${GOAL_PREFIX}paused: the run was interrupted; ${RESUME_HINT}`, "warning")
    } catch (error) {
      failGoal(ctx, error)
    }
  })

  pi.on("context", async (event, ctx) => {
    if (!goal) return undefined
    try {
      const current = goal
      const lastAssistant = event.messages.findLastIndex((message) => message.role === "assistant")
      let changed = false
      const messages = event.messages.filter((message, index) => {
        if (index <= lastAssistant || message.role !== "custom" || message.customType !== GOAL_TYPE) return true
        const generation = (message.details as { generation?: number } | undefined)?.generation
        const keep = current.status === "active" && generation === current.generation
        changed ||= !keep
        return keep
      })
      if (current.status === "active") {
        messages.push({
          role: "user",
          content: [{ type: "text", text: `${GOAL_PREFIX}Active CAFleet goal: ${current.objective}` }],
          timestamp: Date.now(),
        })
        changed = true
      }
      return changed ? { messages } : undefined
    } catch (error) {
      failGoal(ctx, error)
      return undefined
    }
  })
}
