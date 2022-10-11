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

source "$SCRIPTDIR/_isnewversion.sh"
source "$SCRIPTDIR/_npm_published_packages.sh"

ANY_NEW_VERSION=0

for PKGJSON_PATH in "${NPM_PUBLISHED_PACKAGES[@]}"
do
    if isNewVersion $PKGJSON_PATH ; then
        ANY_NEW_VERSION=1
    fi
done

echo "::set-output name=ANY_NEW_VERSION::$ANY_NEW_VERSION"