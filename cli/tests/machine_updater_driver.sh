#!/bin/sh
set -eu
node /test/fixture.mjs &
fixture=$!
trap 'kill "$fixture" 2>/dev/null || true' EXIT
node -e 'const until=Date.now()+10000; (async()=>{for(;;){try{const r=await fetch("http://127.0.0.1:33443/health");if(r.ok)break;}catch{}if(Date.now()>until)process.exit(1);await new Promise(r=>setTimeout(r,50));}})();'
/updater-tests real_container_migration_signed_upgrade_and_rollback --ignored --nocapture

/updater-tests real_companion_handoff_and_rollback --ignored --nocapture
