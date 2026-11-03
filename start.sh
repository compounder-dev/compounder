#!/bin/sh
# The desk runs the funds and is started again if it stops. The server answers the web.
mkdir -p "$DESK_DIR" "$AUTH_DIR"
(while true; do compounder-desk run; sleep 10; done) &
exec compounder-server
