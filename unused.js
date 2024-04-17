#! /usr/bin/env node
let unused = require("./index").findUnusedItems;

const entries = [
  'owa-analytics',
  'owa-analytics-worker',
  'owa-fluent-icons-svg',
  'owa-semantic-overview',
  'owa-fluent-mobile-brand-icons-svg',
  'owa-data-worker-bootstrap',
  'owa-new-user-setup',
  'owa-patch-console',
  'create-clone-opx',
  'sample-query-common',
  'owa-data-worker',
  'pwa-localization',
  'owa-addins-osfruntime-resources',
  'sample-query-field-policy',
  'addison-bootstrap',
  'bookwithme-bootstrap',
  'copilot-hub-bootstrap',
  'eventify-bootstrap',
  'meet-bootstrap',
  'native-host-bootstrap',
  'native-host-deep-bootstrap',
  'oobe-bootstrap',
  'owa-adbar-frame',
  'owa-ads-frame',
  'owa-bookings-bootstrap',
  'owa-bookings-c2-bootstrap',
  'owa-bookings-mobile-bootstrap',
  'owa-bookingsv2-bootstrap',
  'owa-calendar-deeplink-opx-bootstrap',
  'owa-calendar-hosted-bootstrap',
  'owa-calendar-widget',
  'owa-deeplink-bootstrap',
  'owa-findtime-bootstrap',
  'owa-hip-challenge-frame',
  'owa-immersive-bizchat-bootstrap',
  'owa-immersive-bizchat-bootstrap',
  'owa-jit-experience',
  'owa-mail-bootstrap',
  'owa-mail-deeplink-opx-bootstrap',
  'owa-message-recall',
  'owa-opx-app-bootstrap',
  'owa-publishedcalendar-bootstrap',
  'owa-safelink-waitingpage',
  'owa-serviceworker-v2',
  'owa-todo-widget',
  'owa-webpush-serviceworker',
  'places-bootstrap'
]

const { UnusedFinder } = require("./index");
const workers = require("/workspaces/client-web/workers.glob.json");

let report = new UnusedFinder({
  entryPackages: entries,
  filesIgnoredExports: [],
  filesIgnoredImports: [],
  pathsToRead: ["shared", "packages"],
  skippedDirs: [
    // ...workers.files,
    // ...workers.excludedFiles,
  "**/osfruntime_strings.js",
  "**/test/**",
  "**/scripts/**",
  "**/*.Test.ts",
  "**/*.Tests.ts",
  "**/*.d.ts",
  "**/*.d.scss.ts",
  "**/*.d.json.ts",
  "**/*.d.css.ts",
  ],
  skippedItems: [],
  tsConfigPath: "./tsconfig.paths.json",
});
console.log(report.findUnusedItems([]).unusedFiles);
// report.unusedFiles.forEach((file) => {
//   console.log(file);
// });
