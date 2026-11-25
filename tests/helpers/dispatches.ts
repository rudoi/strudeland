/** Count the dispatches `view` receives while `run` runs. */
export function countDispatches(
  view: { dispatch: (...a: never[]) => void },
  run: () => void,
): number {
  const original = view.dispatch;
  let count = 0;
  view.dispatch = (...a: never[]) => {
    count++;
    original.apply(view, a);
  };
  try {
    run();
  } finally {
    view.dispatch = original;
  }
  return count;
}
