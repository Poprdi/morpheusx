#!/usr/bin/env bash
# Full oracle matrix, sequential (net rungs flake under parallel host load).
S=$(cd "$(dirname "$0")" && pwd)
cd "$S"
res=()
run() { ./run_rung.sh "$@" >"$S/logs/$1.run.log" 2>&1; rc=$?; res+=("$1:$([ $rc = 0 ] && echo PASS || echo FAIL)"); echo "${res[-1]}"; }
run rung2    'MORPH_PAL_RUNG2_OK'    'MORPH_PAL_RUNG2_FAIL'
run rung3    'MORPH_PAL_RUNG3_OK'    'MORPH_PAL_RUNG3_FAIL'
run rung4    'MORPH_PAL_RUNG4_OK'    'MORPH_PAL_RUNG4_FAIL'
run envargs  'MORPH_PAL_ENV_OK'      'MORPH_PAL_ENV_FAIL'
run fsdir    'MORPH_PAL_FSDIR_OK'    'MORPH_PAL_FSDIR_FAIL'
run sampler  'MORPH_PAL_SAMPLER_OK'  'MORPH_PAL_SAMPLER_FAIL'
run proc     'MORPH_PAL_PROC_OK'     'MORPH_PAL_PROC_FAIL'
HOSTPORT=45123 run net      'MORPH_PAL_NET_OK'      'MORPH_PAL_NET_FAIL'   net
HOSTPORT=45124 run epoll    'MORPH_PAL_EPOLL_OK'    'MORPH_EPOLL_FAIL'     net
HOSTPORT=45126 run mio2     'MORPH_PAL_MIO_OK'      'MORPH_MIO_FAIL'       net
HOSTPORT=45131 run tokionet 'MORPH_PAL_TOKIONET_OK' 'MORPH_TOKIONET_FAIL'  net
echo "MATRIX: ${res[*]}"
