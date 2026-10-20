let value = 0;
if (true) {
  try {
    (() => [value, after, hoisted()])();
  } catch (error) {
    console.log(error.name);
  }
  value = 2;
}

function hoisted() {
  return value;
}

export const after = 3;
export function read() {
  return [value, after, hoisted()];
}
