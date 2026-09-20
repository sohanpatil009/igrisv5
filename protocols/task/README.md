# protocols/task — durable task contract (v0.1)

TaskNode{id,name,state,attempts,max_retries,checkpoint}. Idempotency key = id.
States: Pending/Running/Paused/Completed/Failed/Cancelled.
Resume = first non-completed in order. Progress = done/total.
