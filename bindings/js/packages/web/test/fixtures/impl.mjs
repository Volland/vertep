// An implementations module: named guards and actions, imported inside the worker.
export const guards = {
  "vt:guard/order/positive": ({ event }) => event.amount > 0,
};

export const actions = {
  "vt:action/order/record": ({ context, event, raise }) => {
    context.total = event.amount;
    raise({ type: "receipt", amount: event.amount });
  },
};
