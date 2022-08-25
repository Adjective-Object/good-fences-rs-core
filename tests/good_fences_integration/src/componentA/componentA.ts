import componentB from '../componentB/componentB';
import helperA1 from './helperA1';
import helperA2 from './helperA2';
import {some, other, stuff} from './helperA1';

if (true) {
    const require = console.log;
    require('foo');
}

export default function componentA() {
    componentB();
    helperA1();
    helperA2();
}
