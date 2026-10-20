# Shared strict admission parser: BusyBox and host awk must agree.
# -v base=... -v architecture=...; output only validated archive rows.
function reject(message) { print "APK catalog: " message > "/dev/stderr"; bad=1; exit 1 }
function hash(value) { return length(value)==64 && value !~ /[^0-9a-f]/ }
NR==1 { if($0!="# nyxid.apk-inputs.v2") reject("unsupported schema"); next }
/^#/ {
    if(rows) reject("header after archives")
    if($2 !~ /^(base-image|arch|refreshed-on|packages|before-sha256|after-sha256)$/ || seen[$2]++) reject("unknown or duplicate header")
    headers++; key=$2
    if(key=="packages") {
        if(NF<3) reject("empty requested packages")
        for(i=3;i<=NF;i++) {
            if($i !~ /^[a-z0-9][a-z0-9+.-]*$/ || (i>3 && $i<=last)) reject("invalid requested packages")
            last=$i
        }
    } else {
        if(NF!=3) reject("invalid header fields")
        if(key=="base-image" && ($3!=base || $3 !~ /^[^ @]+@sha256:[0-9a-f]+$/ || length(substr($3,index($3,"@sha256:")+8))!=64)) reject("base image mismatch")
        if(key=="arch" && ($3!=architecture || $3 !~ /^(x86_64|aarch64)$/)) reject("architecture mismatch")
        if(key=="refreshed-on") refreshed=$3
        if(key=="refreshed-on" && $3 !~ /^[0-9][0-9][0-9][0-9]-[01][0-9]-[0-3][0-9]$/) reject("invalid refresh date")
        if(key ~ /-sha256$/ && !hash($3)) reject("invalid inventory hash")
    }
    next
}
{
    count=split($0,f,"\t")
    if(count!=6 || f[1] !~ /^[a-z0-9][a-z0-9+.-]*$/ || f[2] !~ /^[a-zA-Z0-9][a-zA-Z0-9+_.~-]*$/) reject("invalid archive fields")
    if(f[1]<=prior) reject("duplicate or unsorted archives")
    if(f[3]!=architecture && f[3]!="noarch") reject("foreign archive architecture")
    if(!hash(f[4])) reject("invalid archive checksum")
    expected="/" architecture "/" f[1] "-" f[2] ".apk"
    if(f[6] !~ /^https:\/\/dl-cdn[.]alpinelinux[.]org\/alpine\/v[0-9]+\.[0-9]+\/(main|community)\// || substr(f[6],length(f[6])-length(expected)+1)!=expected) reject("invalid archive URL")
    prefix=substr(f[6],1,length(f[6])-length(expected))
    if(prefix !~ /^https:\/\/dl-cdn[.]alpinelinux[.]org\/alpine\/v[0-9]+\.[0-9]+\/(main|community)$/) reject("unexpected URL path")
    durable="https://github.com/ChronoAIProject/NyxID/releases/download/build-inputs-apk-" refreshed "/" f[4] "-" f[3] "-" f[1] "-" f[2] ".apk"
    if(f[5]!=durable) reject("invalid durable archive URL")
    prior=f[1]; rows++; print $0
}
END {
    if(bad) exit 1
    if(NR<8 || !rows || headers!=6) reject("missing headers or archives")
}
