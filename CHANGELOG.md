# Change Log - @good-fences/api

This log was last generated on Mon, 02 Dec 2024 19:40:25 GMT and should not be manually modified.

<!-- Start content -->

## 0.18.2

Mon, 02 Dec 2024 19:40:25 GMT

### Patches

- remove unbuilt linux-arm-gnueabihf package (mhuan13@gmail.com)

## 0.18.1

Tue, 19 Nov 2024 23:53:48 GMT

### Patches

- Remove duplicate 'testPathPatterns' config property on the napi interface (Maxwell.HuangHobbs@microsoft.com)

## 0.18.0

Thu, 14 Nov 2024 16:54:38 GMT

### Minor changes

- Add support for usage from test files (mhuan13@gmail.com)
- unused_finder: return tagged symbols (mhuan13@gmail.com)
- Expose a jsonschema file for the unused finder config (mhuan13@gmail.com)

## 0.17.0

Thu, 07 Nov 2024 18:37:07 GMT

### Minor changes

- unused_finder: permit type-only exports (Maxwell.HuangHobbs@microsoft.com)
- Rebuild graph traversal for better maintainability, recursive ignore tracking (mhuan13@gmail.com)
- Setup devcontainer, add 'exports' aware cacing resolver (mhuan13@gmail.com)

### Patches

- unittest.yml: remove redundant repo clone in health checks (Maxwell.HuangHobbs@microsoft.com)
- partial refactor of unused_finder to move parsing-related utilities to a submodule (Maxwell.HuangHobbs@microsoft.com)
- add CODEOWNERS file (mhuan13@gmail.com)
- run npm pkg fix (Maxwell.HuangHobbs@microsoft.com)
- remove unused armv7-unknown-linux-gnueabihf from publish pipeline (mhuan13@gmail.com)
- extract packagejson from import_resolver (Maxwell.HuangHobbs@microsoft.com)
- update scripts/pre-pr.sh to run beachball (Maxwell.HuangHobbs@microsoft.com)
- ahashmap+test_tmpdir: add amap2 for creating AHashMap with non-String keys (Maxwell.HuangHobbs@microsoft.com)
- reduce compilation times by breaking up swc_utils (mhuan13@gmail.com)
- cancel old CI jobs when new PR revisions are pushed (Maxwell.HuangHobbs@microsoft.com)
- support heterogenous export maps in packagejson files (mhuan13@gmail.com)
- Add internal utility to check if a file is exported from a package (Maxwell.HuangHobbs@microsoft.com)

## 0.16.0

Tue, 20 Aug 2024 15:59:23 GMT

### Minor changes

- Implement display() for unused report (Maxwell.HuangHobbs@microsoft.com)

## 0.15.0

Mon, 19 Aug 2024 23:19:27 GMT

### Minor changes

- add report_exported_items flag to gate reporting exported items (Maxwell.HuangHobbs@microsoft.com)

### Patches

- upgrade swc, add pretty_assertions (Maxwell.HuangHobbs@microsoft.com)

## 0.14.2

Mon, 19 Aug 2024 23:17:54 GMT

### Patches

- upgrade swc, add pretty_assertions (Maxwell.HuangHobbs@microsoft.com)

## 0.14.1

Wed, 14 Aug 2024 16:58:33 GMT

### Patches

- refactor into multi-crate workspace (Maxwell.HuangHobbs@microsoft.com)

## 0.14.0

Mon, 05 Aug 2024 18:41:53 GMT

### Minor changes

- update yarn + cargo (Maxwell.HuangHobbs@microsoft.com)

## 0.13.2

Tue, 21 May 2024 21:28:51 GMT

### Patches

- Fixed issue preventing unused-finder work on newly-created codespaces (edgarivanv@microsoft.com)

## 0.13.1

Thu, 18 Apr 2024 00:06:46 GMT

### Patches

- Fix UnusedFinder struct not using in-memory representation (edgarivanv@microsoft.com)

## 0.13.0

Tue, 09 Apr 2024 21:53:14 GMT

### Minor changes

- Reverted good-fences resolver function to enable run quick checks on local environments (edgarivanv@microsoft.com)

## 0.12.0

Wed, 17 Jan 2024 00:11:33 GMT

### Minor changes

- Added anyhow and thiserror to do better error handling (edgarivanv@microsoft.com)

## 0.11.0

Tue, 05 Dec 2023 20:57:23 GMT

### Minor changes

- Added support for apple sillicon (edgar21_9@hotmail.com)

### Patches

- Added triples additional property (edgar21_9@hotmail.com)

## 0.10.0

Wed, 01 Nov 2023 15:24:31 GMT

### Minor changes

- Added comments to unused finder struct. Improve validations for bfs_step. Remove rendundant property in unusedFinder struct (edgar21_9@hotmail.com)

## 0.9.0

Tue, 24 Oct 2023 21:41:35 GMT

### Minor changes

- Added struct/class to run unused finder tool with in memory representation (edgar21_9@hotmail.com)

## 0.8.0

Wed, 18 Oct 2023 02:57:33 GMT

### Minor changes

- Added report struct to improve error handling in owa-build (edgar21_9@hotmail.com)

## 0.7.0

Tue, 17 Oct 2023 20:48:23 GMT

### Minor changes

- Added support for `.unusedignore` file (edgar21_9@hotmail.com)

## 0.6.1

Mon, 16 Oct 2023 17:13:19 GMT

### Patches

- Add feature to resolve export-from and track unused items (edgar21_9@hotmail.com)

## 0.6.0

Mon, 09 Oct 2023 19:37:23 GMT

### Minor changes

- Changed logic to find unused exports from sweeping all files to doing tree shaking (edgar21_9@hotmail.com)

## 0.5.1

Wed, 04 Oct 2023 21:00:21 GMT

### Patches

- Added support for files with multiple `.` in the name (edgar21_9@hotmail.com)

## 0.5.0

Mon, 02 Oct 2023 19:18:23 GMT

### Minor changes

- Added metadata struct for exported items (edgar21_9@hotmail.com)

## 0.4.0

Fri, 29 Sep 2023 22:46:14 GMT

### Minor changes

- Upgraded swc (edgar21_9@hotmail.com)

## 0.3.0

Wed, 06 Sep 2023 18:43:57 GMT

### Minor changes

- Added comments to config (edgar21_9@hotmail.com)

## 0.2.0

Fri, 01 Sep 2023 15:09:07 GMT

### Minor changes

- Changed validations done to check if item was being unused (edgar21_9@hotmail.com)
