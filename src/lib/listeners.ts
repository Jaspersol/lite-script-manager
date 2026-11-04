import { listen, type EventCallback, type EventName, type UnlistenFn } from "@tauri-apps/api/event";

export function createListenerGroup() {
  const unlisteners: UnlistenFn[] = [];
  let disposed = false;

  function add<T>(event: EventName, handler: EventCallback<T>) {
    void listen<T>(event, handler)
      .then((off) => {
        if (disposed) off();
        else unlisteners.push(off);
      })
      .catch((error: unknown) => {
        console.error(`Event listener failed: ${String(event)}`, error);
      });
  }

  function dispose() {
    disposed = true;
    for (const off of unlisteners.splice(0)) off();
  }

  return { add, dispose };
}
