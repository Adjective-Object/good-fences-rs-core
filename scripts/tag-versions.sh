#!/bin/bash
# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.
#
# This script is used in the github actions workflows
# to tag and push the current npm package versions

set -eo

SCRIPTDIR=`dirname $0`
ROOT=`realpath $SCRIPTDIR/..`

source "$SCRIPTDIR/_isnewversion.sh"
source "$SCRIPTDIR/_npm_published_packages.sh"

for PKGJSON_PATH in "${NPM_PUBLISHED_PACKAGES[@]}"
do
    if isNewVersion $PKGJSON_PATH ; then
        JSON_VERSION=`jq .version $PKGJSON_PATH`
        CLEAN_VERSION=$(echo $JSON_VERSION | tr -d \")

        JSON_NAME=`jq .name $PKGJSON_PATH`
        CLEAN_NAME=$(echo $JSON_NAME | tr -d \")

        TAG="$CLEAN_NAME/v$CLEAN_VERSION"

        if git tag | grep '^'"$TAG"'$'; then
            echo "tag $TAG already exists! failed to tag"
            exit 1
        else 
            echo "tagging" $TAG
            git tag "$TAG"
        fi
    fi
done