#! /usr/bin/env node
let unused = require('./index').findUnusedItems;

const entry = [
    '**/owa-calendar-deeplink-opx-bootstrap/**',
    '**/owa-publishedcalendar-bootstrap/**',
    '**/owa-mail-bootstrap/**',
    '**/owa-deeplink-bootstrap/**',
    '**/owa-mail-deeplink-opx-bootstrap/**',
    '**/meet-bootstrap/**',
    '**/owa-message-recall/**',
    '**/native-host-bootstrap/**',
    '**/native-host-deep-bootstrap/**',
    '**/owa-tokenprovider/**',
    '**/owa-bookings-bootstrap/**',
    '**/owa-bookingsv2-bootstrap/**',
    '**/owa-bookings-mobile-bootstrap/**',
    '**/owa-bookings-c2-bootstrap/**',
    '**/owa-serviceworker-v2/**',
    '**/owa-webpush-serviceworker/**',
    '**/owa-findtime-bootstrap/**',
    '**/owa-jit-experience/**',
    '**/eventify-bootstrap/**',
    '**/owa-opx-app-bootstrap/**',
    '**/owa-safelink-waitingpage/**',
    '**/owa-calendar-widget/**',
    '**/owa-todo-widget/**',
    '**/bookwithme-bootstrap/**',
    '**/owa-ads-frame/**',
    '**/owa-adbar-frame/**',
    '**/owa-fluent-icons-svg/App/**',
    '**/oobe-bootstrap/**',
    '**/addison-bootstrap/**',
    '**/places-bootstrap/**',
    '**/owa-immersive-bizchat-bootstrap/**',
    '**/owa-immersive-bizchat-bootstrap/**',
];


unused(
    ['packages', 'shared'], 
    './tsconfig.paths.json', 
    [
        ...entry,
        "**/__mocks__/**",
        "**/test/**",
        "**/owa-fluent-icons-svg/**",
        "**/__generated__/**",
        "**/*.g.ts",
        "**/owa-addins-osfruntime-resources/**",
        "**/owa-fluent-mobile-brand-icons-svg/**"
    ],
    [".*Props$", ".*Tests$", "^(?i)test_.*"]
);