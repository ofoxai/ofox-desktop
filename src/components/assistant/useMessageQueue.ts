import { useMemo, useRef, useState } from "react";

/** The host's UTF-8 message limit, also used for unsent queue entries. */
export const MAX_MESSAGE_BYTES = 128_000;
export const MAX_QUEUED_MESSAGES = 10;

export interface QueuedMessage {
  id: string;
  text: string;
}

/** One claimed entry. Pausing revokes admission, not an already sent run. */
export interface QueueTicket {
  sessionId: string;
  message: QueuedMessage;
  cancelled: boolean;
}

export interface MessageQueue {
  messages: QueuedMessage[];
  paused: boolean;
  /** A successful, saved run end or an explicit resume permits one send. */
  ready: boolean;
  sending: QueueTicket | null;
}

const emptyQueue = (): MessageQueue => ({
  messages: [],
  paused: false,
  ready: false,
  sending: null,
});

export function validQueuedText(text: string) {
  return Boolean(
    text.trim() && new TextEncoder().encode(text).length <= MAX_MESSAGE_BYTES,
  );
}

/**
 * Per-conversation, memory-only ordinary messages. No runtime followUp/steer
 * or automatic retries: each admitted entry becomes a separate ordinary run.
 * The ref changes synchronously, so Stop/remove/leave beat delayed admission
 * even before React commits their render.
 */
export function useMessageQueue() {
  const [queues, setQueues] = useState(new Map<string, MessageQueue>());
  const current = useRef(queues);
  const controls = useMemo(() => {
    const put = (sid: string, queue: MessageQueue | null) => {
      const next = new Map(current.current);
      if (queue) next.set(sid, queue);
      else next.delete(sid);
      current.current = next;
      setQueues(next);
    };
    const pause = (sid: string) => {
      const queue = current.current.get(sid) ?? emptyQueue();
      if (queue.sending) queue.sending.cancelled = true;
      // Even an already-paused queue gets a new snapshot: leaving again
      // must revoke a Resume that is still waiting for the history read.
      put(sid, { ...queue, paused: true, ready: false });
    };
    const clear = (sid?: string) => {
      if (sid !== undefined) {
        const queue = current.current.get(sid);
        if (!queue) return;
        if (queue.sending) queue.sending.cancelled = true;
        put(sid, null);
      } else {
        for (const queue of current.current.values())
          if (queue.sending) queue.sending.cancelled = true;
        current.current = new Map();
        setQueues(current.current);
      }
    };
    return {
      get: (sid: string) => current.current.get(sid),
      clear,
      pause,
      pauseAll: () => {
        for (const sid of current.current.keys()) pause(sid);
      },
      /** An explicit non-queue send never implicitly resumes old entries. */
      manualRun: (sid: string) => {
        if (current.current.get(sid)?.messages.length) pause(sid);
        else clear(sid);
      },
      enqueue: (sid: string, text: string) => {
        const queue = current.current.get(sid) ?? emptyQueue();
        if (
          !validQueuedText(text) ||
          queue.messages.length >= MAX_QUEUED_MESSAGES
        )
          return false;
        put(sid, {
          ...queue,
          messages: [...queue.messages, { id: crypto.randomUUID(), text }],
        });
        return true;
      },
      edit: (sid: string, id: string, text: string) => {
        const queue = current.current.get(sid);
        if (
          !queue ||
          !validQueuedText(text) ||
          queue.sending?.message.id === id ||
          !queue.messages.some((message) => message.id === id)
        )
          return false;
        pause(sid);
        put(sid, {
          ...current.current.get(sid)!,
          messages: queue.messages.map((message) =>
            message.id === id ? { ...message, text } : message,
          ),
        });
        return true;
      },
      remove: (sid: string, id: string) => {
        const queue = current.current.get(sid);
        if (!queue || queue.sending?.message.id === id) return;
        const messages = queue.messages.filter((message) => message.id !== id);
        put(sid, messages.length ? { ...queue, messages } : null);
      },
      completed: (sid: string) => {
        const queue = current.current.get(sid);
        if (queue && !queue.paused) put(sid, { ...queue, ready: true });
      },
      resume: (sid: string) => {
        const queue = current.current.get(sid);
        if (queue?.messages.length && !queue.sending)
          put(sid, { ...queue, paused: false, ready: true });
      },
      claim: (sid: string): QueueTicket | null => {
        const queue = current.current.get(sid);
        if (
          !queue?.messages.length ||
          queue.paused ||
          !queue.ready ||
          queue.sending
        )
          return null;
        const ticket: QueueTicket = {
          sessionId: sid,
          message: queue.messages[0],
          cancelled: false,
        };
        put(sid, { ...queue, ready: false, sending: ticket });
        return ticket;
      },
      settle: (ticket: QueueTicket, sent: boolean) => {
        const queue = current.current.get(ticket.sessionId);
        // A reset/removal never gets resurrected by an old promise.
        if (queue?.sending !== ticket) return;
        const messages = sent
          ? queue.messages.filter((message) => message.id !== ticket.message.id)
          : queue.messages;
        put(
          ticket.sessionId,
          messages.length
            ? {
                ...queue,
                messages,
                sending: null,
                paused: queue.paused || !sent,
                ready: sent && queue.ready,
              }
            : null,
        );
      },
    };
  }, []);
  return { queues, controls };
}
