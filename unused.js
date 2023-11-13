#! /usr/bin/env node
let unused = require("./index").findUnusedItems;

const entries = [
  "owa-addins-osfruntime-resources",
  "owa-analytics",
  "owa-analytics-worker",
  "owa-data-worker",
  "owa-data-worker-bootstrap",
  "owa-fluent-icons-svg",
  "owa-fluent-mobile-brand-icons-svg",
  "addison-bootstrap",
  "bookwithme-bootstrap",
  "create-clone-opx",
  "eventify-bootstrap",
  "meet-bootstrap",
  "native-host-bootstrap",
  "native-host-deep-bootstrap",
  "oobe-bootstrap",
  "owa-adbar-frame",
  "owa-ads-frame",
  "owa-bookings-bootstrap",
  "owa-bookings-c2-bootstrap",
  "owa-bookings-mobile-bootstrap",
  "owa-bookingsv2-bootstrap",
  "owa-calendar-deeplink-opx-bootstrap",
  "owa-calendar-widget",
  "owa-data-worker-bootstrap",
  "owa-deeplink-bootstrap",
  "owa-findtime-bootstrap",
  "owa-immersive-bizchat-bootstrap",
  "owa-jit-experience",
  "owa-mail-bootstrap",
  "owa-mail-deeplink-opx-bootstrap",
  "owa-message-recall",
  "owa-new-user-setup",
  "owa-opx-app-bootstrap",
  "owa-publishedcalendar-bootstrap",
  "owa-safelink-waitingpage",
  "owa-semantic-overview",
  "owa-serviceworker-v2",
  "owa-todo-widget",
  "owa-tokenprovider",
  "owa-webpush-serviceworker",
  "places-bootstrap",
  "pwa-localization",
  "sample-query-common",
  "sample-query-field-policy",
];

const { UnusedFinder } = require("./index");
const workers = require("/workspaces/client-web/workers.glob.json");

const config = {
  entryPackages: entries,
  filesIgnoredExports: [],
  filesIgnoredImports: [],
  pathsToRead: ["shared", "packages"],
  skippedDirs: [
    // ...workers.files,
    // ...workers.excludedFiles,
    "**/osfruntime_strings.js",
    "**/scripts/**",
    "**/*.d.ts",
    "**/*.d.scss.ts",
    "**/*.d.json.ts",
    "**/*.d.css.ts",
  ],
  skippedItems: [],
  tsConfigPath: "./tsconfig.paths.json",
};

let report = new UnusedFinder(config).findUnusedItems([]);
// let report = unused(config);
// console.log(report);
console.log(report.testOnlyUsedFiles);
