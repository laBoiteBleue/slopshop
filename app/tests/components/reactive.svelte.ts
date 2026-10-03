// A test helper: an object whose changes the components follow (a `$bindable` prop that the
// app keeps in its own `$state`), and that the test can read back.
export function reactive<T extends object>(value: T): T {
  const state = $state(value);
  return state;
}
