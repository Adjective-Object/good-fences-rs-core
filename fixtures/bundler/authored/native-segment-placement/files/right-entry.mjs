import { named, local, identity, value, bump } from './right.mjs';
console.log('right', named(), named.name, local, value, identity.token);
console.log('right:bump', bump(), value);
try {
  value = 99;
} catch (error) {
  console.log(error.name);
}
