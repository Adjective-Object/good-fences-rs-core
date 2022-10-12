# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

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