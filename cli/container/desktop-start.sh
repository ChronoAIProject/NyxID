#!/bin/sh
# Shared by self-hosted setup and sealed managed startup.
set -eu
umask 077
: "${XAUTHORITY:=/var/lib/nyxid-machine/desktop/Xauthority}"
export XAUTHORITY
mkdir -p /var/lib/nyxid-machine/desktop /home/browser /home/devbrowser
chmod 0711 /var/lib/nyxid-machine /var/lib/nyxid-machine/desktop
# Serialize the two fixed displays, including preflight and cookie creation.
# Do not pass the lock to long-lived desktop children.
exec 9>/var/lib/nyxid-machine/desktop/start.lock
if ! flock -n 9; then
    printf '%s\n' 'Machine desktop stage=display_preflight reason=display_in_use_or_unverified' >&2
    exit 1
fi
# /tmp survives a managed container restart but its private PID namespace does
# not. A retained X lock can name a PID allocated to the *new* Xvfb, which then
# mistakes itself for the previous server. Remove only a fully identified,
# browser-owned orphan pair, before allocating any desktop children. A live PID,
# kernel socket, incomplete pair or uncertain identity must remain untouched.
python3 - <<'DISPLAYS'
import os, pwd, re, stat, sys

def refuse():
    print('Machine desktop stage=display_preflight reason=display_in_use_or_unverified', file=sys.stderr)
    raise SystemExit(1)

def snapshot(path):
    try:
        return os.lstat(path)
    except FileNotFoundError:
        return None

def identity(info):
    return None if info is None else (info.st_dev, info.st_ino, info.st_mode,
        info.st_uid, info.st_gid, info.st_nlink, info.st_size, info.st_mtime_ns, info.st_ctime_ns)

def bound(number):
    names = {f'/tmp/.X11-unix/X{number}', f'@/tmp/.X11-unix/X{number}'}
    with open('/proc/net/unix') as sockets:
        return any(len(fields := line.split()) == 8 and fields[-1] in names for line in sockets)

def absent(pid):
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return True
    except PermissionError:
        pass
    return False

try:
    try:
        os.mkdir('/tmp/.X11-unix', 0o1777)
        os.chmod('/tmp/.X11-unix', 0o1777)
    except FileExistsError:
        pass
    for directory in ('/tmp', '/tmp/.X11-unix'):
        info = os.lstat(directory)
        if (not stat.S_ISDIR(info.st_mode) or info.st_uid != 0 or info.st_gid != 0
                or stat.S_IMODE(info.st_mode) != 0o1777):
            refuse()
    orphans = []
    for number, user in ((99, 'browser'), (100, 'devbrowser')):
        owner = pwd.getpwnam(user)
        lock, socket = f'/tmp/.X{number}-lock', f'/tmp/.X11-unix/X{number}'
        left, right = snapshot(lock), snapshot(socket)
        if bound(number):
            refuse()
        if left is None and right is None:
            continue
        if left is None or right is None:
            refuse()
        if (not stat.S_ISREG(left.st_mode) or not stat.S_ISSOCK(right.st_mode)
                or left.st_nlink != 1 or right.st_nlink != 1 or left.st_size != 11
                or left.st_uid != owner.pw_uid or right.st_uid != owner.pw_uid
                or left.st_gid != owner.pw_gid or right.st_gid != owner.pw_gid
                or stat.S_IMODE(left.st_mode) != 0o444 or stat.S_IMODE(right.st_mode) != 0o777):
            refuse()
        fd = os.open(lock, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, 'rb') as stream:
            if identity(os.fstat(stream.fileno())) != identity(left):
                refuse()
            value = stream.read(64)
        if not re.fullmatch(rb' *[1-9][0-9]*\n', value):
            refuse()
        pid = int(value)
        if pid > 2147483647 or not absent(pid):
            refuse()
        orphans.append((number, owner, lock, socket, left, right, pid))
    for number, owner, lock, socket, left, right, pid in orphans:
        if (identity(snapshot(lock)) != identity(left) or identity(snapshot(socket)) != identity(right)
                or bound(number) or not absent(pid)):
            refuse()
        child = os.fork()
        if child == 0:
            try:
                os.setgroups([])
                os.setgid(owner.pw_gid)
                os.setuid(owner.pw_uid)
                os.unlink(socket)
                os.unlink(lock)
                os._exit(0)
            except OSError:
                os._exit(1)
        if os.waitpid(child, 0)[1] != 0:
            refuse()
except (OSError, KeyError, ValueError):
    refuse()
DISPLAYS
MACHINE_CHILDREN=
MACHINE_START_WAIT=0
cleanup_desktop_start() {
    for MACHINE_CHILD in $MACHINE_CHILDREN; do
        kill "$MACHINE_CHILD" 2>/dev/null || true
    done
    for MACHINE_CHILD in $MACHINE_CHILDREN; do
        wait "$MACHINE_CHILD" 2>/dev/null || true
    done
}
trap cleanup_desktop_start EXIT
trap 'exit 1' HUP INT TERM
desktop_fail() {
    printf 'Machine desktop stage=%s reason=%s\n' "$1" "$2" >&2
    exit 1
}
check_desktop_child() {
    if ! kill -0 "$1" 2>/dev/null; then
        if wait "$1"; then MACHINE_CHILD_STATUS=0; else MACHINE_CHILD_STATUS=$?; fi
        desktop_fail "$2" "child_exited_$MACHINE_CHILD_STATUS"
    fi
}
# CHOWN is available to the supervisor; FOWNER/DAC_OVERRIDE are not. Apply
# modes while the supervisor owns each directory, then hand it to its user.
chown root:root /home/browser /home/devbrowser
chmod 0700 /home/browser /home/devbrowser
chown browser:browser /home/browser
chown devbrowser:devbrowser /home/devbrowser
# Each browser has an independent display and authentication cookie.
export NYXID_DEV_DISPLAY=:100
export NYXID_DEV_XAUTHORITY=/var/lib/nyxid-machine/desktop/dev-Xauthority
for MACHINE_DESKTOP in secure dev; do
    if [ "$MACHINE_DESKTOP" = secure ]; then
        MACHINE_USER=browser
        MACHINE_DISPLAY=:99
        MACHINE_AUTH="$XAUTHORITY"
    else
        MACHINE_USER=devbrowser
        MACHINE_DISPLAY="$NYXID_DEV_DISPLAY"
        MACHINE_AUTH="$NYXID_DEV_XAUTHORITY"
    fi
    # Cookies are temporary. Recreate them without needing DAC override to
    # read a prior browser-owned mode-0600 cookie after a container restart.
    rm -f "$MACHINE_AUTH"
    MACHINE_COOKIE=$(openssl rand -hex 16)
    printf 'add %s MIT-MAGIC-COOKIE-1 %s\n' "$MACHINE_DISPLAY" "$MACHINE_COOKIE" | xauth -f "$MACHINE_AUTH" source -
    unset MACHINE_COOKIE
    chown "root:$MACHINE_USER" "$MACHINE_AUTH"
    chmod 0640 "$MACHINE_AUTH"
    runuser -u "$MACHINE_USER" -- env -i PATH=/usr/bin:/bin HOME="/home/$MACHINE_USER" DISPLAY="$MACHINE_DISPLAY" XAUTHORITY="$MACHINE_AUTH" \
        Xvfb "$MACHINE_DISPLAY" -screen 0 1280x800x24 -nolisten tcp -auth "$MACHINE_AUTH" 9>&- &
    MACHINE_XVFB_PID=$!
    MACHINE_CHILDREN="$MACHINE_CHILDREN $MACHINE_XVFB_PID"
    while :; do
        check_desktop_child "$MACHINE_XVFB_PID" "${MACHINE_DESKTOP}_xvfb"
        if runuser -u "$MACHINE_USER" -- env -i PATH=/usr/bin:/bin HOME="/home/$MACHINE_USER" DISPLAY="$MACHINE_DISPLAY" XAUTHORITY="$MACHINE_AUTH" xdpyinfo >/dev/null 2>&1; then
            break
        fi
        [ "$MACHINE_START_WAIT" -lt 200 ] || desktop_fail "${MACHINE_DESKTOP}_xvfb" readiness_timeout
        MACHINE_START_WAIT=$((MACHINE_START_WAIT + 1))
        sleep 0.1
    done
    check_desktop_child "$MACHINE_XVFB_PID" "${MACHINE_DESKTOP}_xvfb"
    if [ "$MACHINE_DESKTOP" = dev ]; then
        MACHINE_DEV_XVFB_PID=$MACHINE_XVFB_PID
        runuser -u devbrowser -- env -i PATH=/usr/bin:/bin HOME=/home/devbrowser DISPLAY="$MACHINE_DISPLAY" XAUTHORITY="$MACHINE_AUTH" \
            openbox 9>&- >/dev/null 2>&1 &
        MACHINE_DEV_SESSION_PID=$!
        MACHINE_CHILDREN="$MACHINE_CHILDREN $MACHINE_DEV_SESSION_PID"
    else
        MACHINE_SECURE_XVFB_PID=$MACHINE_XVFB_PID
    fi
done
unset MACHINE_DESKTOP MACHINE_USER MACHINE_DISPLAY MACHINE_AUTH
runuser -u browser -- rm -f /home/browser/.nyxid-desktop/session-bus.address
# Openbox waits for the display without exposing Xauthority to command children.
runuser -u browser -- env -i PATH=/usr/bin:/bin HOME=/home/browser DISPLAY=:99 XAUTHORITY="$XAUTHORITY" \
    /usr/local/bin/nyxid-desktop-session 9>&- >/dev/null 2>&1 &
# This is the browser user's private D-Bus session, never an agent environment.
MACHINE_SESSION_PID=$!
MACHINE_CHILDREN="$MACHINE_CHILDREN $MACHINE_SESSION_PID"
while ! runuser -u browser -- test -s /home/browser/.nyxid-desktop/session-bus.address; do
    check_desktop_child "$MACHINE_SECURE_XVFB_PID" secure_xvfb
    check_desktop_child "$MACHINE_DEV_XVFB_PID" dev_xvfb
    check_desktop_child "$MACHINE_DEV_SESSION_PID" dev_window_manager
    check_desktop_child "$MACHINE_SESSION_PID" secure_accessibility
    [ "$MACHINE_START_WAIT" -lt 200 ] || desktop_fail secure_accessibility readiness_timeout
    MACHINE_START_WAIT=$((MACHINE_START_WAIT + 1))
    sleep 0.1
done
runuser -u browser -- cat /home/browser/.nyxid-desktop/session-bus.address > /var/lib/nyxid-machine/desktop/session-bus.address
chmod 0600 /var/lib/nyxid-machine/desktop/session-bus.address
check_desktop_child "$MACHINE_SECURE_XVFB_PID" secure_xvfb
check_desktop_child "$MACHINE_DEV_XVFB_PID" dev_xvfb
check_desktop_child "$MACHINE_DEV_SESSION_PID" dev_window_manager
check_desktop_child "$MACHINE_SESSION_PID" secure_accessibility
# Successful children remain under the existing Tini/PID-namespace lifecycle.
trap - EXIT HUP INT TERM
exec 9>&-
