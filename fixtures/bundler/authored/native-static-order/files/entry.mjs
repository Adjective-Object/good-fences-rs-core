#!/usr/bin/env node
'use strict';
import {call as invoke, Type as Constructor} from './pure.mjs';
import {globalName as local} from './a.mjs';
console.log(invoke.name, Constructor.name, typeof globalName, local, this === undefined);
export {invoke as default};
