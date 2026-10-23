#!/usr/bin/env bash
# Asserts on the logs of a debug self-test run. Usage: selftest-check.sh <app-log> <server-log>
app=$1; srv=$2; fail=0
want() { grep -qE "$2" "$1" && echo "ok   $3" || { echo "FAIL $3"; fail=1; }; }
want "$app" 'address_bar_swap profile=Work' 'address bar swaps profile'
want "$app" 'undo profile=Default' 'undo swap'
want "$app" 'link_click_swap profile=Work' 'link click swaps profile'
want "$app" 'redirect_no_swap profile=Default' 'redirect does not swap'
want "$app" 'palette webview exists=true' 'palette webview'
want "$app" 'panel webview exists=true' 'panel webview'
want "$app" '\[selftest\] done' 'self-test finished'
want "$srv" "cookie=None" 'cookie isolation (clean profile saw no cookie)'
want "$srv" "ADRESULT.*'hidden': 'true'" 'ad blocker hides cosmetic element'
if grep -q panicked "$app"; then echo "FAIL panic"; fail=1; fi
exit $fail
