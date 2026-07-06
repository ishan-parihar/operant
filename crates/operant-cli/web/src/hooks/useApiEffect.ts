/**
 * useApiEffect — a thin wrapper around useEffect that adds AbortController
 * to async API calls. Prevents stale responses from overwriting fresh
 * state when the user navigates away or switches profiles mid-fetch.
 *
 * (iter-134 — closes the ponytail-audit gap "no AbortController on ANY
 * of the ~30+ useEffect fetches; stale responses after profile switch
 * / rapid nav will overwrite fresh state".)
 *
 * Usage:
 *   useApiEffect(
 *     (signal) => api.getSessions(signal),
 *     (sessions) => setSessions(sessions),
 *     [],  // deps
 *   );
 *
 * The hook creates an AbortController, passes its signal to the fetch
 * function, guards the callback with !aborted, and aborts on cleanup.
 */
import { useEffect, useRef } from "react";

export function useApiEffect<T>(
  fetcher: (signal: AbortSignal) => Promise<T>,
  onSuccess: (data: T) => void,
  deps: React.DependencyList = [],
  options?: { onError?: (e: unknown) => void },
) {
  // Keep the fetcher/onSuccess refs stable so we don't re-run on every render.
  const fetcherRef = useRef(fetcher);
  const onSuccessRef = useRef(onSuccess);
  const onErrorRef = useRef(options?.onError);
  fetcherRef.current = fetcher;
  onSuccessRef.current = onSuccess;
  onErrorRef.current = options?.onError;

  useEffect(() => {
    const controller = new AbortController();
    fetcherRef
      .current(controller.signal)
      .then((data) => {
        if (!controller.signal.aborted) {
          onSuccessRef.current(data);
        }
      })
      .catch((e) => {
        if (!controller.signal.aborted && onErrorRef.current) {
          onErrorRef.current(e);
        }
      });
    return () => controller.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);
}
