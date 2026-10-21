trace.push('hub');
const identity = {};
let state = 0;
export function bump() {
  state++;
  return {identity, count: state, read: () => state};
}
