import { Plugin } from "@opencode/plugin"
import { spawn, type ChildProcess } from "node:child_process"
import { accessSync, constants, readFileSync, statSync } from "node:fs"
import { createInterface } from "node:readline"
import { fileURLToPath } from "node:url"

const RECORD_PREFIX = "[cafleet listen record] "
const CLOSED_PREFIX = "[cafleet listen closed] "
const GOAL_PREFIX = "[cafleet goal] "
const CONTINUE_PREFIX = `${GOAL_PREFIX}Continue working toward the active goal: `
const MEMBER_REFUSAL = "cafleet tools run only in a Director session"
const SETUP_FIX = "run 'cafleet setup --coding-agent opencode'"
const ENTRY_KEY = "cafleet-goal"
const GENERATION_KEY = "cafleetGoalGeneration"
const IDLE_LIMIT = 3
const CONTINUATION_LIMIT = 50

type Status = "active" | "paused" | "complete"

interface Goal {
  objective: string
  status: Status
  generation: number
  continuations: number
  idle_streak: number
}

interface TurnState {
  continuationTurn: boolean
  progressed: boolean
}

interface ToolContext {
  readonly sessionID: string
  readonly agent: string
  readonly messageID: string
  readonly id: string
}

const result = (fields: Record<string, unknown>) => ({ content: JSON.stringify(fields) })

function recordedExecutable(): string {
  const record = fileURLToPath(new URL("./cafleet.executable", import.meta.url))
  let target: string
  try {
    target = readFileSync(record, "utf8").trim()
  } catch (error) {
    throw new Error(`executable record ${record}: ${(error as Error).message}; ${SETUP_FIX}`)
  }
  try {
    if (!statSync(target).isFile()) throw new Error(`${target} is not a file`)
    accessSync(target, constants.X_OK)
  } catch (error) {
    throw new Error(`executable record ${record}: ${(error as Error).message}; ${SETUP_FIX}`)
  }
  return target
}

export default Plugin.define({
  id: "cafleet",
  async setup(ctx) {
    const isMember = Boolean(process.env.CAFLEET_MEMBER_ID)
    const receivers = new Map<string, ChildProcess>()
    const stoppedReceivers = new WeakSet<ChildProcess>()
    const continuationGenerations = new Map<string, number>()
    const turns = new Map<string, TurnState>()
    const touched = new Set<string>()

    const storageKey = (sessionID: string) => `goal/${sessionID}`
    const generationKey = (sessionID: string) => `goal-generation/${sessionID}`
    const readGoal = async (sessionID: string) =>
      (await ctx.storage.get(storageKey(sessionID))) as unknown as Goal | undefined

    async function writeGoal(sessionID: string, goal: Goal) {
      await ctx.storage.set(storageKey(sessionID), goal as never)
      await ctx.session.instructions.entry.put({
        sessionID,
        key: ENTRY_KEY,
        value:
          `CAFleet goal (${goal.status}): ${goal.objective}\n` +
          "While the goal is active, keep working toward it; call cafleet_goal_progress after each step " +
          "and cafleet_goal_complete when it is achieved.",
      })
    }

    async function nextGeneration(sessionID: string) {
      const issued = ((await ctx.storage.get(generationKey(sessionID))) as unknown as number | undefined) ?? 0
      await ctx.storage.set(generationKey(sessionID), (issued + 1) as never)
      return issued + 1
    }

    async function clearGoal(sessionID: string) {
      await nextGeneration(sessionID)
      await ctx.storage.remove(storageKey(sessionID))
      await ctx.session.instructions.entry.remove({ sessionID, key: ENTRY_KEY })
    }

    async function touch(sessionID: string) {
      if (touched.has(sessionID)) return
      touched.add(sessionID)
      const goal = await readGoal(sessionID)
      if (goal?.status === "active") await writeGoal(sessionID, goal)
    }

    async function notice(sessionID: string, text: string, resume: boolean) {
      await ctx.session.synthetic({ sessionID, text: GOAL_PREFIX + text, delivery: "queue", resume })
    }

    async function queueContinuation(sessionID: string, goal: Goal) {
      const next = { ...goal, continuations: goal.continuations + 1 }
      await writeGoal(sessionID, next)
      const item = await ctx.session.synthetic({
        sessionID,
        text:
          `${CONTINUE_PREFIX}${goal.objective}. ` +
          "Call cafleet_goal_progress after each step and cafleet_goal_complete when it is achieved.",
        delivery: "queue",
        metadata: { [GENERATION_KEY]: goal.generation },
      })
      continuationGenerations.set(item.id, goal.generation)
    }

    async function pause(sessionID: string, goal: Goal) {
      await writeGoal(sessionID, { ...goal, status: "paused", generation: await nextGeneration(sessionID) })
    }

    function requireDirector() {
      if (isMember) throw new Error(MEMBER_REFUSAL)
    }

    async function authorize(tool: string, context: ToolContext) {
      await ctx.permission.assert({
        sessionID: context.sessionID,
        agent: context.agent,
        source: { type: "tool", messageID: context.messageID, id: context.id },
        action: "cafleet",
        resources: [tool],
      } as never)
    }

    function stopReceiver(child: ChildProcess) {
      stoppedReceivers.add(child)
      child.kill()
    }

    function startReceiver(sessionID: string, directorID: number) {
      const child = spawn(recordedExecutable(), ["message", "listen", String(directorID)], {
        stdio: ["ignore", "pipe", "pipe"],
      })
      receivers.set(sessionID, child)
      let sawClosed = false
      let lastStderr = ""
      let delivery: Promise<unknown> = Promise.resolve()
      createInterface({ input: child.stdout! }).on("line", (line) => {
        if (!line) return
        sawClosed ||= line.startsWith('{"event":"closed"')
        delivery = delivery.then(() =>
          ctx.session
            .synthetic({ sessionID, text: RECORD_PREFIX + line, delivery: "queue" })
            .catch(() => stopReceiver(child)),
        )
      })
      createInterface({ input: child.stderr! }).on("line", (line) => {
        if (line.trim()) lastStderr = line.trim()
      })
      child.on("exit", (code, signal) => {
        if (receivers.get(sessionID) === child) receivers.delete(sessionID)
        if (sawClosed || stoppedReceivers.has(child) || signal === "SIGTERM") return
        delivery = delivery.then(() =>
          ctx.session
            .synthetic({
              sessionID,
              text: `${CLOSED_PREFIX}exit ${code ?? signal}: ${lastStderr}`,
              delivery: "queue",
            })
            .catch(() => undefined),
        )
      })
      return child
    }

    await ctx.tool.transform((editor) => {
      editor.add({
        name: "cafleet_listen",
        description:
          "Start CAFleet Director reception for this session: every broker record addressed to the Director " +
          "arrives as its own turn beginning '[cafleet listen record]'.",
        input: {
          type: "object",
          properties: { director_id: { type: "integer", description: "The Director's literal member ID." } },
          required: ["director_id"],
          additionalProperties: false,
        },
        execute: async (input, context) => {
          if (isMember) return result({ ok: false, error: MEMBER_REFUSAL })
          await authorize("cafleet_listen", context)
          await touch(context.sessionID)
          const existing = receivers.get(context.sessionID)
          if (existing && existing.exitCode === null && existing.signalCode === null) {
            return result({ ok: true, pid: existing.pid, sessionID: context.sessionID, existing: true })
          }
          try {
            const child = startReceiver(context.sessionID, (input as { director_id: number }).director_id)
            return result({ ok: true, pid: child.pid, sessionID: context.sessionID })
          } catch (error) {
            return result({ ok: false, error: (error as Error).message })
          }
        },
      })
      editor.add({
        name: "cafleet_goal_status",
        description: "Report the CAFleet goal of this session.",
        input: { type: "object", properties: {}, additionalProperties: false },
        execute: async (_input, context) => {
          if (isMember) return result({ ok: false, error: MEMBER_REFUSAL })
          await authorize("cafleet_goal_status", context)
          await touch(context.sessionID)
          return { content: JSON.stringify((await readGoal(context.sessionID)) ?? null) }
        },
      })
      editor.add({
        name: "cafleet_goal_progress",
        description: "Report a completed step toward the active CAFleet goal of this session.",
        input: {
          type: "object",
          properties: { note: { type: "string" } },
          required: ["note"],
          additionalProperties: false,
        },
        execute: async (_input, context) => {
          if (isMember) return result({ ok: false, error: MEMBER_REFUSAL })
          await authorize("cafleet_goal_progress", context)
          await touch(context.sessionID)
          const goal = await readGoal(context.sessionID)
          if (!goal) return result({ ok: false, error: "no CAFleet goal is set for this session" })
          await writeGoal(context.sessionID, { ...goal, idle_streak: 0 })
          turns.set(context.sessionID, { ...(turns.get(context.sessionID) ?? { continuationTurn: false }), progressed: true })
          return result({ ok: true })
        },
      })
      editor.add({
        name: "cafleet_goal_complete",
        description: "Mark the active CAFleet goal of this session as achieved.",
        input: {
          type: "object",
          properties: { summary: { type: "string" } },
          required: ["summary"],
          additionalProperties: false,
        },
        execute: async (_input, context) => {
          if (isMember) return result({ ok: false, error: MEMBER_REFUSAL })
          await authorize("cafleet_goal_complete", context)
          await touch(context.sessionID)
          const goal = await readGoal(context.sessionID)
          if (!goal) return result({ ok: false, error: "no CAFleet goal is set for this session" })
          await writeGoal(context.sessionID, {
            ...goal,
            status: "complete",
            generation: await nextGeneration(context.sessionID),
          })
          turns.set(context.sessionID, { ...(turns.get(context.sessionID) ?? { continuationTurn: false }), progressed: true })
          return result({ ok: true })
        },
      })
    })

    await ctx.command.transform((editor) => {
      editor.add({
        name: "cafleet-goal",
        description: "Set a CAFleet goal (text), or report status / pause / resume / clear it.",
        execute: async ({ sessionID, prompt }) => {
          requireDirector()
          await touch(sessionID)
          const argument = (prompt.text ?? "").trim()
          const goal = await readGoal(sessionID)
          if (argument === "status" || argument === "") {
            await notice(sessionID, `status: ${JSON.stringify(goal ?? null)}`, true)
            return
          }
          if (["pause", "resume", "clear"].includes(argument) && !goal) {
            throw new Error("no CAFleet goal is set for this session")
          }
          if (argument === "pause") return pause(sessionID, goal!)
          if (argument === "clear") return clearGoal(sessionID)
          if (argument === "resume") {
            const resumed: Goal = {
              ...goal!,
              status: "active",
              generation: await nextGeneration(sessionID),
              continuations: 0,
              idle_streak: 0,
            }
            return queueContinuation(sessionID, resumed)
          }
          const created: Goal = {
            objective: argument,
            status: "active",
            generation: await nextGeneration(sessionID),
            continuations: 0,
            idle_streak: 0,
          }
          return queueContinuation(sessionID, created)
        },
      })
    })

    if (isMember) return () => undefined

    await ctx.session.hook("context", async (input) => {
      const goal = await readGoal(input.sessionID)
      input.messages = input.messages.filter((message) => {
        if (message.role !== "user") return true
        const continuation = message.content.some(
          (part) => part.type === "text" && part.text.startsWith(CONTINUE_PREFIX),
        )
        if (!continuation) return true
        if (goal?.status !== "active") return false
        const generation =
          (message.id === undefined ? undefined : continuationGenerations.get(message.id)) ??
          (message.metadata?.[GENERATION_KEY] as number | undefined)
        return generation === undefined || generation === goal.generation
      })
    })

    const events = ctx.event.subscribe({
      types: ["session.inbox.delivered", "session.execution.succeeded", "session.execution.interrupted"],
    })
    void (async () => {
      for await (const event of events) {
        const { type, data } = event as unknown as {
          type: string
          data: { sessionID: string; inboxID?: string; reason?: string }
        }
        await touch(data.sessionID)
        if (type === "session.inbox.delivered") {
          if (data.inboxID !== undefined && continuationGenerations.has(data.inboxID)) {
            turns.set(data.sessionID, { continuationTurn: true, progressed: false })
          }
          continue
        }
        const turn = turns.get(data.sessionID)
        turns.delete(data.sessionID)
        const goal = await readGoal(data.sessionID)
        if (goal?.status !== "active") continue
        if (type === "session.execution.interrupted") {
          if (data.reason === "user") await pause(data.sessionID, goal)
          continue
        }
        const idle = turn?.continuationTurn && !turn.progressed ? goal.idle_streak + 1 : goal.idle_streak
        if (idle >= IDLE_LIMIT) {
          await pause(data.sessionID, { ...goal, idle_streak: idle })
          await notice(
            data.sessionID,
            `paused: no progress in ${IDLE_LIMIT} consecutive continuation turns; ` +
              "the operator resumes it with /cafleet-goal resume",
            false,
          )
          continue
        }
        if (goal.continuations >= CONTINUATION_LIMIT) {
          await pause(data.sessionID, { ...goal, idle_streak: idle })
          await notice(
            data.sessionID,
            `paused: reached ${CONTINUATION_LIMIT} continuations; the operator resumes it with /cafleet-goal resume`,
            false,
          )
          continue
        }
        await queueContinuation(data.sessionID, { ...goal, idle_streak: idle })
      }
    })()

    return () => {
      for (const child of receivers.values()) stopReceiver(child)
      receivers.clear()
    }
  },
})
