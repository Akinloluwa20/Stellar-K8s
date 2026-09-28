# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |

## Reporting a Vulnerability

### Security Contact

For security vulnerabilities, please **do not** use the public issue tracker. Instead, report security issues privately to:

**Email**: security@stellar.org  
**Subject**: [stellar-k8s] Security Vulnerability Report

### What to Include

When reporting a vulnerability, please include:

1. **Description**: Clear description of the vulnerability
2. **Steps to Reproduce**: Detailed reproduction steps
3. **Impact Assessment**: Potential impact and affected components
4. **Suggested Fix**: If you have a proposed solution
5. **Disclosure Timeline**: Your preferred disclosure timeline

### Response Process

1. **Acknowledgment**: We'll acknowledge receipt within 48 hours
2. **Investigation**: Initial assessment within 5 business days
3. **Resolution**: Security fixes prioritized based on severity
4. **Disclosure**: Coordinated disclosure after fix is available

## Security Measures

### Dependency Security

This project implements comprehensive dependency security monitoring:

- **Automated Scanning**: `cargo audit` and `cargo deny` in CI
- **Automated Updates**: Dependabot (`.github/dependabot.yml`); review and merge process in [`docs/security/dependency-updates.md`](docs/security/dependency-updates.md)
- **License Compliance**: Strict allowlist of permitted licenses
- **Vulnerability Tracking**: All security advisories reviewed and documented
- **Supply Chain Security**: Dependency provenance verification
- **Secret Scanning**: Gitleaks + custom pattern-based secret detection

### Secret Scanning

Multi-layered secret detection:

1. **Gitleaks** — Pattern-based scanning for AWS keys, GitHub tokens, PEM keys, Stellar seeds, connection strings
2. **Custom scanner** (`scripts/check-secrets.sh`) — Stellar-specific patterns, shell echo hygiene, GitHub Actions secret safety, Dockerfile hygiene, Rust source literals
3. **Pre-commit hooks** — Local scanning before commit

Configuration: `.gitleaks.toml`
Custom scanner: `scripts/check-secrets.sh`

### License Compliance

- **cargo-deny**: Enforces approved license allowlist (MIT, Apache-2.0, BSD-2/3, ISC, MPL-2.0, etc.)
- **License headers**: Automated enforcement of Apache-2.0 headers on Rust, Shell, and YAML files
- **Third-party tracking**: `THIRD_PARTY_LICENSES.md` verified in CI

Denied licenses: Any license not in the explicit allowlist in `deny.toml`.

### Security Scanning

```bash
# Run all security checks
make security-all

# Individual checks
cargo audit                    # Vulnerability scan
cargo deny check               # License + bans + advisories
gitleaks detect --config .gitleaks.toml  # Secret scanning
bash scripts/check-secrets.sh  # Custom secret patterns
make check-license-headers     # License header enforcement
```

### Build Security

- **Hardened Profiles**: Security-optimized release builds
- **Symbol Stripping**: Debug symbols removed from production builds
- **Panic Handling**: Abort on panic for production deployments
- **Link-Time Optimization**: Enhanced security through LTO

### Runtime Security

- **TLS by Default**: All network communication encrypted
- **Certificate Validation**: Strict certificate validation
- **Access Controls**: Kubernetes RBAC integration
- **Audit Logging**: Comprehensive security event logging

### Container Security

- **Minimal Base Images**: Distroless base images
- **Non-Root Execution**: Containers run as non-root user
- **Read-Only Root**: Immutable container filesystem
- **Security Contexts**: Restricted security contexts

## Security Testing

### Automated Tests

```bash
# Run security audit
cargo deny check
cargo audit

# Check for outdated dependencies
cargo outdated

# Run secret scanning
./scripts/check-secrets.sh
```

### Manual Security Reviews

- Quarterly dependency review
- Annual penetration testing (recommended)
- Code review for security-sensitive changes
- Security architecture review for major features

## Security Configuration

### Environment Hardening

```yaml
# Kubernetes Security Context
securityContext:
  runAsNonRoot: true
  runAsUser: 65534
  readOnlyRootFilesystem: true
  allowPrivilegeEscalation: false
  capabilities:
    drop: ["ALL"]
```

### Network Security

- **Network Policies**: Restrict ingress/egress traffic
- **TLS Everywhere**: All inter-service communication encrypted
- **Certificate Rotation**: Automated certificate lifecycle management

## Compliance

### Standards

- **CIS Kubernetes Benchmark**: Aligned with CIS recommendations
- **NIST Cybersecurity Framework**: Risk management approach
- **OWASP Top 10**: Web application security considerations

### Auditing

- Security events logged and monitored
- Compliance reporting available
- Regular security assessments

## Security Resources

### Documentation

- [Dependency Security Audit](./DEPENDENCY_SECURITY_AUDIT.md)
- [Secret Scanning Script](./scripts/check-secrets.sh)
- [Deny Configuration](./deny.toml)

### Tools

- `cargo audit` - Vulnerability scanning
- `cargo deny` - Policy enforcement  
- `cargo outdated` - Dependency updates
- Custom security checks in CI/CD

### External Resources

- [RustSec Advisory Database](https://rustsec.org/)
- [Kubernetes Security Best Practices](https://kubernetes.io/docs/concepts/security/)
- [Stellar Security Guidelines](https://developers.stellar.org/docs/encyclopedia/security)

---

**Note**: This security policy is reviewed quarterly and updated as needed. Last updated: $(date +%Y-%m-%d)