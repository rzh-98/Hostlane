import { callHostlane } from '@/app/daemon'
import { createStore } from '@/lib/store'
import type { HostlaneError, HostlaneEvent } from '@shared/ipc'
import type { Job } from '@shared/protocol/Job'
import type { JobLogs } from '@shared/protocol/JobLogs'
import type { JobsList } from '@shared/protocol/JobsList'

const HISTORY_LIMIT = 200
/** Lines fetched when a job is first shown. */
const TAIL_LINES = 500
/** Output kept per job in the view; older output is dropped (it stays on the server). */
const VIEW_CAP = 256 * 1024

export interface JobOutput {
  text: string
  /** Byte offset on the server up to which `text` reaches. */
  nextOffset: number
  /** Output before `text` exists on the server. */
  clipped: boolean
  state: 'loading' | 'loaded' | 'error'
  error?: string
}

interface JobsState {
  jobs: Record<string, Job>
  outputs: Record<string, JobOutput>
  loaded: boolean
  error: HostlaneError | null
}

export const jobsStore = createStore<JobsState>(null, {
  jobs: {},
  outputs: {},
  loaded: false,
  error: null
})

export async function loadJobs(): Promise<void> {
  const reply = await callHostlane<JobsList>('jobs.list', { limit: HISTORY_LIMIT })
  if (!reply.ok) {
    jobsStore.set({ error: reply.error, loaded: true })
    return
  }
  const jobs: Record<string, Job> = {}
  for (const job of reply.result.jobs) jobs[job.id] = job
  jobsStore.set({ jobs, loaded: true, error: null })
}

function setJob(job: Job): void {
  jobsStore.set((state) => ({ jobs: { ...state.jobs, [job.id]: job } }))
}

function setOutput(id: string, output: JobOutput): void {
  jobsStore.set((state) => ({ outputs: { ...state.outputs, [id]: output } }))
}

const fetching = new Set<string>()

/** Loads the tail of a job's output, or what was added since the last read. */
export async function readJobOutput(id: string): Promise<void> {
  if (fetching.has(id)) return
  fetching.add(id)
  try {
    const known = jobsStore.get().outputs[id]
    const fresh = !known || known.state === 'error'
    if (fresh) setOutput(id, { text: '', nextOffset: 0, clipped: false, state: 'loading' })
    const params = fresh ? { id, tail_lines: TAIL_LINES } : { id, offset: known.nextOffset }
    const reply = await callHostlane<JobLogs>('jobs.logs', params)
    if (!reply.ok) {
      setOutput(id, {
        ...(known ?? { text: '', nextOffset: 0, clipped: false }),
        state: 'error',
        error: reply.error.message
      })
      return
    }
    const chunk = reply.result
    setJob(chunk.job)
    let text = (fresh ? '' : known.text) + chunk.data
    let clipped = fresh ? chunk.offset > 0 : known.clipped
    if (text.length > VIEW_CAP) {
      text = text.slice(text.length - VIEW_CAP)
      clipped = true
    }
    setOutput(id, { text, nextOffset: chunk.next_offset, clipped, state: 'loaded' })
  } finally {
    fetching.delete(id)
  }
}

/** Asks the server for the job's state now. */
export async function refreshJob(id: string): Promise<HostlaneError | null> {
  const reply = await callHostlane<Job>('jobs.status', { id })
  if (!reply.ok) return reply.error
  setJob(reply.result)
  return null
}

export async function cancelJob(id: string): Promise<HostlaneError | null> {
  const reply = await callHostlane<Job>('jobs.cancel', { id })
  if (!reply.ok) return reply.error
  setJob(reply.result)
  return null
}

export function handleJobEvent(event: HostlaneEvent): void {
  if (event.type === 'job') setJob(event.job)
  else if (event.type === 'subscribed' || event.type === 'lagged') void loadJobs()
}

export function resetJobs(): void {
  fetching.clear()
  jobsStore.reset()
}
