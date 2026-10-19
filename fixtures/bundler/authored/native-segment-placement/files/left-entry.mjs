import { named, local, identity, value, bump } from './left.mjs';
console.log('left', named(), named.name, local, value, identity.token);
console.log('left:bump', bump(), value);
try {
  value = 99;
} catch (error) {
  console.log(error.name);
}
