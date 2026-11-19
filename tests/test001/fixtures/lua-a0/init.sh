#!/bin/busybox sh
export PATH=/bin LC_ALL=C TZ=UTC TERM=dumb
umask 022
case "$LIBC" in
    musl|glibc) ;;
    *) echo 'A0_SETUP_ERROR libc'; /bin/busybox poweroff -f; exit 1 ;;
esac
cd "/$LIBC" || exit 1
echo "A0_SAMPLE_BEGIN libc=$LIBC scope=lua-only"
if [ -e test.txt ]; then
    echo 'A0_SETUP_ERROR preexisting-test.txt'
    /bin/busybox poweroff -f
    exit 1
fi
./busybox sh ./lua_testcode.sh
script_rc=$?
echo "A0_GROUP_SCRIPT_RC $script_rc"
if [ -e test.txt ]; then
    echo 'A0_POST_TEST_TXT PRESENT'
else
    echo 'A0_POST_TEST_TXT ABSENT'
fi
./lua -e 'return -1'
echo "A0_RETURN_MINUS_ONE_RC $?"
./lua -e 'error("A0_EXPECTED_ERROR")'
echo "A0_EXPLICIT_ERROR_RC $?"
echo 'A0_MISSING_INPUT_PROBE_BEGIN'
./test.sh __a0_missing.lua
echo "A0_WRAPPER_MISSING_INPUT_RC $?"
echo 'A0_MISSING_INPUT_PROBE_END'
echo "A0_SAMPLE_END libc=$LIBC"
/bin/busybox poweroff -f
