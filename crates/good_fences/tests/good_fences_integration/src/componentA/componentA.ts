import componentB from '../componentB/componentB'; // INVALID IMPORT
import helperA1 from './helperA1';
import helperA2 from './helperA2';
import {some, other, stuff} from './helperA1';
require('./helperA1');
import('./helperA1');

if (true) {
    const require = console.log;
    require('foo');
}

export default function componentA() {
    componentB();
    helperA1();
    helperA2();
}
