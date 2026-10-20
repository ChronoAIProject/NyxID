"""Ephemeral local-fixture CA and a distinct, CA-signed TLS server leaf."""
import argparse
import ipaddress
from pathlib import Path
import subprocess


def generate(directory, gateway):
    addresses = sorted({'127.0.0.1', str(ipaddress.ip_address(gateway))})
    ca, ca_key = directory / 'ca.pem', directory / 'ca.key'
    leaf, leaf_key = directory / 'tls.pem', directory / 'tls.key'
    ca_config, leaf_config = directory / 'ca.cnf', directory / 'tls.cnf'
    ca_config.write_text('''[req]
distinguished_name = dn
x509_extensions = ca
prompt = no
[dn]
CN = NyxID disposable local fixture CA
[ca]
basicConstraints = critical,CA:TRUE,pathlen:0
keyUsage = critical,keyCertSign,cRLSign
subjectKeyIdentifier = hash
''')
    leaf_config.write_text('''[req]
distinguished_name = dn
prompt = no
[dn]
CN = NyxID disposable local fixture server
[server]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature,keyEncipherment
extendedKeyUsage = serverAuth
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid,issuer
subjectAltName = ''' + ','.join('IP:' + value for value in addresses) + '\n')

    def openssl(*arguments):
        subprocess.run(['openssl', *map(str, arguments)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    openssl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-set_serial', '1',
            '-config', ca_config, '-keyout', ca_key, '-out', ca)
    openssl('req', '-new', '-newkey', 'rsa:2048', '-nodes', '-config', leaf_config,
            '-keyout', leaf_key, '-out', directory / 'tls.csr')
    openssl('x509', '-req', '-in', directory / 'tls.csr', '-CA', ca, '-CAkey', ca_key,
            '-set_serial', '2', '-days', '1', '-sha256', '-extfile', leaf_config,
            '-extensions', 'server', '-out', leaf)
    for key in (ca_key, leaf_key):
        key.chmod(0o600)
    openssl('verify', '-CAfile', ca, '-purpose', 'sslserver', leaf)
    return ca, leaf, leaf_key


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('gateway')
    arguments = parser.parse_args()
    generate(arguments.directory, arguments.gateway)
