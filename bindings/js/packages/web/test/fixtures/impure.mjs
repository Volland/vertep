// A guard that breaks the purity contract: it answers differently on every call.
let calls = 0;
export const guards = {
  "vt:guard/order/positive": () => calls++ === 0,
};
export const actions = {
  "vt:action/order/record": () => undefined,
};
