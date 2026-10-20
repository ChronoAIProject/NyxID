# source timestamp, checked-on date, official bundle/checksum URLs, exact SHA-256.
BEGIN {FS="\t"}
function reject() {bad=1; exit 1}
NR==1 {if($0!="# nyxid.apt-transport-ca.v1") reject(); next}
NR==2 {
    if(NF!=5 || length($1)!=20 || $1!~/^[0-9TZ:-]+$/ || length($2)!=10 || $2!~/^[0-9-]+$/ ||
       $3!="https://curl.se/ca/cacert.pem" || $4!="https://curl.se/ca/cacert.pem.sha256" ||
       length($5)!=64 || $5!~/^[0-9a-f]+$/) reject()
    digest=$5
}
END {if(bad || NR!=2) exit 1; print digest}
