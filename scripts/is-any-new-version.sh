# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.
#
# This script is used in the github actions workflows
# to determine if the current git HEAD is a new version
# compared to the previous version
#
# If so, it will print a string recognized by github
# actions, used in the publish-prod.yml script to
# determine if we should cut a new version

set -eo

SCRIPTDIR=`dirname $0`
ROOT=`realpath $SCRIPTDIR/..`

function isNewVersion() {
    _PKGJSON_PATH=$1

    _REPO_ROOT=`git rev-parse --show-toplevel`

    _CURRENT_PKGJSON=`jq .version "$_REPO_ROOT/$_PKGJSON_PATH"`
    if [[ "$_CURRENT_PKGJSON" == "" ]]; then
        echo "failed to parse git ref HEAD" >&2
        exit 1
    fi
    _PRIOR_PKGJSON=$(git show HEAD^1:"$_PKGJSON_PATH" | jq .version)
    if [[ "$_PRIOR_PKGJSON" == "" ]]; then
        echo "failed to parse git ref HEAD" >&2
        exit 1
    fi

    echo -n "checking $_PKGJSON_PATH for version change .. "

    if [[ "$_CURRENT_PKGJSON" != "$_PRIOR_PKGJSON" ]]; then
        echo "bump $_PRIOR_PKGJSON -> $_CURRENT_PKGJSON"
        return 0
    else
        echo "no bump $_PRIOR_PKGJSON"
        return 1
    fi
}
NPM_PUBLISHED_PACKAGES=(
    "package.json" 
    "npm/darwin-x64/package.json"
    "npm/win32-x64-msvc/package.json"
    "npm/linux-x64-gnu/package.json"
)

ANY_NEW_VERSION=0

for PKGJSON_PATH in "${NPM_PUBLISHED_PACKAGES[@]}"
do
    if isNewVersion $PKGJSON_PATH ; then
        ANY_NEW_VERSION=1
    fi
done

echo "::set-output name=ANY_NEW_VERSION::$ANY_NEW_VERSION"