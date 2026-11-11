// A guard that kills the worker thread, to observe WorkerTerminated.
export const guards = {
  "vt:guard/order/positive": () => process.exit(3),
};
export const actions = {
  "vt:action/order/record": () => undefined,
};
