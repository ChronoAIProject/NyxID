# NyxID — Terms of Use

**Last updated:** 11 May 2026

> **IMPORTANT NOTICE:** PLEASE READ THESE TERMS OF USE CAREFULLY BEFORE ACCESSING OR USING THE NYXID APPLICATION. BY ACCESSING OR USING THE APP, YOU CONFIRM THAT YOU HAVE READ, UNDERSTOOD, AND AGREE TO BE LEGALLY BOUND BY THESE TERMS. PAY PARTICULAR ATTENTION TO **SECTIONS 4 (USER-OPERATED AI AGENTS), 6 (SECURITY DISCLAIMERS), 12 (LIMITATION OF LIABILITY), AND 15 (ARBITRATION).** IF YOU DO NOT AGREE TO THESE TERMS, DO NOT ACCESS OR USE THE APP.

These NyxID Terms of Use ("**Terms**") constitute a legal agreement between you ("**User**" or "**you**") and ChronoAI Pte. Ltd. ("**we**," "**us**," or "**ChronoAI**") (the "**Agreement**"). These Terms apply when you visit or interact with the NyxID application, engage with our customer support, interact with us on social media, or otherwise communicate with us. By accessing or using the App, you agree to be bound by these Terms.

## 1. CONFIRMATION AND ACCEPTANCE OF THESE TERMS

### 1.1 Entire Agreement and Scope of Applicability

These Terms of Use ("**Terms**"), together with the Privacy Policy and any other documents expressly incorporated by reference herein (collectively, the "**Agreement**"), constitute the entire and exclusive agreement between you ("**User**" or "**you**") and ChronoAI Pte. Ltd. ("**ChronoAI**", "**we**", "**us**", or "**our**") concerning your access to and use of the NyxID identity and credential infrastructure (the "**App**") — comprising the NyxID mobile application, web application, command-line interface ("**CLI**"), Node Agent daemon, and SDK client libraries — and all related services, features, and content provided by ChronoAI (collectively, the "**Services**"). References to the App in these Terms apply equally to each of these surfaces unless the context requires otherwise.

This Agreement supersedes all prior or contemporaneous communications, proposals, or agreements, whether electronic, oral, or written, relating to the subject matter hereof. For the avoidance of doubt, this Agreement does not extend to, nor does ChronoAI assume any responsibility or liability for, services developed, operated, or provided by third parties, even if accessible or linked through the App.

### 1.2 Account Credentials and Shared Authentication

Where NyxID utilises a shared or unified authentication system with any affiliated or related application ("**Partner Application**"), the User acknowledges and agrees that:

- authentication assertions, OAuth/OIDC tokens, and your NyxID identifier may be shared with the Partner Application to enable single sign-on; your NyxID password, MFA Secrets, and stored API Keys and Tokens are never shared with Partner Applications;
- your use of the App is governed exclusively by this Agreement and the related Privacy Policy;
- your use of any Partner Application is governed exclusively by the separate terms of service and privacy policy published by the operator of that Partner Application;
- ChronoAI Pte. Ltd. operates all current Partner Applications and is the sole controller of personal data processed through them. If, at any future time, shared authentication creates a Joint Controller relationship with a separate legal entity under applicable law (including Article 26 of the GDPR), ChronoAI will disclose the essence of that arrangement, including the allocation of responsibility for Data Subject Access Requests and credential security, in the Privacy Policy and any applicable Joint Controller Agreement.

### 1.3 Acceptance of Terms

By accessing or using any or all of the App, you expressly acknowledge that (i) you have read and understood these Terms; (ii) you agree to be bound by these Terms; and (iii) you are legally competent to enter into these Terms. If you do not agree to be bound by these Terms or any updates or modifications to these Terms, you may not access or use our App.

### 1.4 Modifications to these Terms

ChronoAI reserves the right to amend these Terms from time to time, including to reflect changes in applicable law, regulatory guidance, or our Services. Material changes will be communicated by revising the "Last updated" date, by displaying a notice within the App, or by contacting you directly where required by applicable law. Your continued use of the App following any modification constitutes acceptance of the revised Terms. If you do not agree to any modification, you must cease using the App.

### 1.5 Privacy Policy

For an explanation of how we collect, use and disclose information from our users, please see our Privacy Policy at [/privacy](/privacy). You acknowledge and agree that your use of the App is subject to, and that we may collect, use and/or disclose your information (including any personal data you provide to us) in accordance with our Privacy Policy.

### 1.6 Eligibility

To be eligible to use the App, you must:

- be at least eighteen (18) years of age and legally competent to enter into binding agreements under the laws of your jurisdiction;
- not be a resident of, or located in, a jurisdiction subject to applicable trade embargoes, UN Security Council Resolutions, or sanctions regimes (including those administered by OFAC, HM Treasury, or the UN Sanctions Committee);
- not be listed on any sanctions list, including the UN Security Council Consolidated List, the U.S. Specially Designated Nationals List, or any equivalent list maintained by a relevant authority;
- not use the Services if doing so would violate any applicable law or regulation in your jurisdiction.

If you are accessing the Services on behalf of a legal entity, you represent and warrant that the entity is duly incorporated and that you are duly authorised to act on its behalf and bind it to this Agreement. ChronoAI reserves the right to gate registration (for example, behind an invitation programme or waitlist), modify eligibility criteria, and restrict access at any time.

### 1.7 Organizations and Org Admins

The App supports Organizations as a deployment model in which a single legal entity provisions and administers NyxID accounts, credentials, services, nodes, and other resources on behalf of its members. Each Organization may designate one or more **Org Admins** who hold elevated authority to provision and revoke member access, manage org-owned services and credentials, and act on behalf of the Organization within the App. Org Admins are bound by these Terms in their own capacity and as agents of the Organization.

Members of an Organization acknowledge that their account is administered by the Organization and that Org Admins may have visibility into credentials, services, and audit records associated with the Organization. Organizations are responsible for their own internal governance, including the consequences of administrator changes (for example, ensuring that registration tokens or other delegated authority granted by a removed administrator are revoked in a timely manner). ChronoAI's role is limited to applying the access-control rules configured within the App.

### 1.8 Service Accounts

The App allows you to provision Service Accounts — non-human identities that hold credentials and tokens for machine-to-machine use. Where you create or operate a Service Account, you remain the responsible User for all activity performed under that Service Account, including its compliance with these Terms. References in these Terms to your use of the App include use by any Service Account you have created or that operates on your behalf.

## 2. DEFINITIONS

For the purposes of this Agreement, the following terms shall have the meanings set out below:

- **"Alert"** means a message displayed on the App's interface providing suggestions or notifications to Users regarding authentication requests, approval decisions, or system events.
- **"API Keys and Tokens"** means authentication credentials, API keys, OAuth tokens, SSH Certificates, and other similar access credentials that you store within the App for the purpose of proxied service access.
- **"App"** means the NyxID identity and credential infrastructure offered by ChronoAI, comprising the mobile application, web application, CLI, Node Agent, SDK, and all updates, upgrades, and versions thereof. References to the App apply equally to each of these surfaces unless the context requires otherwise.
- **"Channel Bot"** means a third-party messaging-platform bot (for example, a Telegram bot, a Lark / Feishu bot, or a Discord bot) that you register with the App so that inbound messages addressed to that bot are routed to agents or other services you have authorised.
- **"CLI"** means the `nyxid` command-line interface tool distributed by ChronoAI.
- **"Approval Request"** means a push notification or messaging-platform message sent to you requiring your approval or denial before a proxied credential request is executed.
- **"ChronoAI" / "we" / "us" / "our"** means ChronoAI Pte. Ltd., a company incorporated in Singapore, and its successors and assigns.
- **"Credential Proxy"** means the functionality by which the App injects your stored API Keys and Tokens into outbound requests to third-party services on your behalf.
- **"GDPR"** means the General Data Protection Regulation (EU) 2016/679, as amended or replaced from time to time.
- **"Intellectual Property"** means all patents, copyrights, trademarks, trade secrets, database rights, design rights, and all other intellectual property rights, whether registered or unregistered.
- **"Local Agent"** means software operated by the User on their own hardware — including the Node Agent and the CLI — which may store API Keys and Tokens locally without transmitting them to ChronoAI's servers.
- **"MFA Secrets"** means multi-factor authentication seeds, time-based one-time password (TOTP) secrets, and similar data used to generate authentication codes.
- **"Node Agent"** means the NyxID daemon software that you may install on infrastructure you operate (for example, as a launchd, systemd, or Docker service), and that maintains a persistent connection to ChronoAI's servers in order to proxy requests using credentials held locally on that infrastructure.
- **"OAuth/OIDC"** means the OAuth 2.0 and OpenID Connect authentication protocols.
- **"Organization"** means a legal entity that has provisioned a NyxID account in the form of an Organization and on behalf of which one or more Users have been provisioned access by an Org Admin.
- **"Org Admin"** means a User designated by an Organization with elevated authority to administer that Organization's accounts, credentials, services, nodes, and other resources within the App.
- **"Partner Application"** means any application developed or operated by a third party (or by ChronoAI) that integrates with the App via "Sign in with NyxID" or similar shared authentication functionality.
- **"PDPA"** means the Personal Data Protection Act 2012 (Singapore), as amended or replaced from time to time, including applicable subsidiary legislation and guidelines issued by the Personal Data Protection Commission (PDPC).
- **"Personal Data"** has the meaning given to it under applicable data protection law, and includes information from which you can be identified directly or indirectly.
- **"Reply Token"** means a single-use, short-lived token issued by the App to authorise a specific reply to a particular inbound message through a Channel Bot or similar integration.
- **"SDK"** means the NyxID software development kits and client libraries published by ChronoAI for the purpose of integrating Partner Applications with the App (for example, "Sign in with NyxID" and related OAuth/OIDC flows).
- **"Service Account"** means a non-human identity provisioned within the App that holds credentials and tokens for machine-to-machine use under the authority of a responsible User.
- **"Services"** means all features, functions, tools, and content made available through the App, as further described in Section 3.
- **"SSH Certificate"** means a short-lived cryptographically signed certificate issued by the App for the purpose of authenticating remote server access.
- **"User" / "you" / "your"** means any natural person who accesses or uses the App, including both registered account holders and visitors. Where a Service Account acts within the App, references to the User include the responsible natural person who provisioned or operates that Service Account.

## 3. SERVICES AND FUNCTIONALITIES

### 3.1 Service Description

NyxID is an identity and secure credential proxy service. The App enables Users to create an account, securely store API Keys and Tokens for remote third-party services, and have those credentials injected into outbound requests via the Credential Proxy functionality. NyxID is network and identity infrastructure; it does not itself incorporate or perform artificial intelligence or machine-learning inference. Where the App routes requests to third-party AI providers via the Model Context Protocol ("**MCP**") or LLM gateway functionality, those providers process your prompts and responses under their own terms; NyxID acts solely as a routing, identity, and credential-injection layer between you and your chosen AI provider. See Section 4 for the position on user-operated AI agents.

### 3.2 Core Functionalities

The App provides the following core Services:

- **Credential Storage and Proxy:** You may store API Keys and Tokens within the App (encrypted at rest). NyxID proxies requests to third-party services by injecting your stored credentials on your behalf. Proxied request and response bodies are buffered in memory for the Approval Request flow only and are not written to disk or persistently logged.
- **Local Agent (Node Agent and CLI):** You may optionally run the Node Agent daemon (installed via launchd, systemd, or Docker, optionally in multi-profile mode) or the CLI on your own infrastructure, allowing credentials to remain on your hardware and never be transmitted to ChronoAI's servers. The Node Agent maintains a persistent connection to ChronoAI's servers for proxy routing and supports failover across multiple nodes you have registered.
- **Approval Interface:** The mobile App and supported messaging platforms serve as approval interfaces, enabling you to approve, deny, or revoke access requests via push notification (iOS/Android) or messaging-platform message before each proxied request is executed.
- **OAuth/OIDC Login Provider and SDK:** NyxID can act as an OAuth/OIDC login provider, enabling third-party developers to integrate "Sign in with NyxID" into their Partner Applications. ChronoAI publishes the SDK to support these integrations; developer obligations are addressed in Section 5.7.
- **SSH Certificate Issuance:** The App can issue short-lived SSH Certificates for authenticating remote server access.

### 3.3 Service Evolution

ChronoAI reserves the right to introduce, modify, suspend, or discontinue any Service or feature at any time. Where changes materially affect your use of the Services, ChronoAI will use reasonable endeavours to provide prior notice. Your continued use of the App following any change constitutes your acceptance of the modified Services.

## 4. USER-OPERATED AI AGENTS ("BYOK")

### 4.1 No Inherent AI in the App

NyxID does not itself incorporate artificial intelligence or machine-learning features. The App is identity and credential-broker infrastructure. Any artificial intelligence used in connection with NyxID is supplied and operated by you ("Bring Your Own Key" / BYOK).

### 4.2 User-Operated AI Agents

You may use third-party AI agents (for example, Claude Code, Codex, OpenClaw, or similar) to interact with the App. Such AI agents act under your authority and using credentials you supply, including scoped API keys issued through your NyxID account. ChronoAI does not control, supervise, or assume responsibility for AI agents operated by you or on your behalf.

### 4.3 Responsibility for AI Agent Actions

You are solely responsible for all actions performed by any AI agent acting under your account or API key, including the agent's compliance with these Terms and with the terms of any third-party services accessed via the Credential Proxy. ChronoAI accepts no liability for the outputs, errors, omissions, or unauthorised actions of AI agents operated by you or on your behalf, to the maximum extent permitted by applicable law.

### 4.4 No Automated Decision-Making by ChronoAI with Legal Effect

ChronoAI does not use the App to make automated decisions that produce legal effects concerning you or similarly significantly affect you. Where the App incorporates automated controls (for example, rate limiting, abuse detection, or session termination on suspected compromise), those controls are deterministic security features applied uniformly to all Users, not AI-driven assessments of you as an individual. If you believe an automated control has affected you in error, you may contact ChronoAI at **contact@chrono-ai.fun**.

## 5. USER RIGHTS AND OBLIGATIONS

### 5.1 User Rights

Subject to these Terms, ChronoAI grants you the following rights with respect to your data and account:

- to access, correct, or request deletion of your personal data at any time in accordance with the Privacy Policy;
- to revoke any OAuth consent, approval grant, or API key at any time via the App;
- to choose where credentials are stored — on ChronoAI's servers or on your own hardware via the Local Agent;
- to disconnect social logins, messaging-platform integrations, and push notification services at any time;
- to export your data in a portable format where technically feasible.

### 5.2 Device Security Obligations

You are solely responsible for maintaining the security of your device(s) used to access the App. You agree to:

- use device-level security measures appropriate to the sensitivity of credentials stored on the device (for example, screen lock, biometric authentication, or device encryption, where supported by your device);
- keep your device's operating system and the App updated to the latest version;
- immediately notify ChronoAI at **contact@chrono-ai.fun** if you suspect your device has been lost, stolen, or compromised;
- not jailbreak, root, or otherwise modify your device in a manner that circumvents security controls;
- not install or permit the installation of software that may intercept, monitor, or tamper with App communications or stored credentials;
- where you run the Node Agent or the CLI on infrastructure you operate, keep that host patched and protected against unauthorised access, safeguard any registration tokens, signing secrets, and locally stored credentials, and promptly revoke and rotate any of the foregoing on suspected compromise.

### 5.3 Credential and Account Security Obligations

You are responsible for:

- maintaining the confidentiality of your account credentials (username, password, and MFA Secrets);
- not sharing your account credentials with any third party;
- using strong, unique passwords and enabling multi-factor authentication for your NyxID account;
- promptly revoking any API Keys or Tokens that you believe have been compromised;
- ensuring that all credentials stored within the App are used only for lawful purposes and in accordance with the terms and conditions of the respective third-party service providers.

ChronoAI shall have no liability for any loss or damage arising from your failure to maintain the security of your account credentials or device.

### 5.4 Compliance with Laws

You represent and warrant that you will comply with all applicable laws, regulations, and policies of your country of nationality and/or country of residence in connection with your use of the App. You shall not use the App for any unlawful purpose or through any unlawful means.

### 5.5 Prohibited Activities

You agree not to engage in any of the following activities in connection with your use of the App:

- accessing or attempting to access another User's account, credentials, or data without authorisation;
- using the App to proxy, store, or inject credentials for illegal, unauthorised, or malicious purposes;
- using automated programs, bots, web crawlers, scraping tools, or similar technologies to extract data from or interfere with the App;
- attempting to reverse engineer, decompile, disassemble, or otherwise derive the source code of the App;
- uploading, transmitting, or storing malware, viruses, or other malicious code through the App;
- conducting penetration testing, vulnerability scanning, or any security testing of the App or ChronoAI's infrastructure without prior written authorisation;
- impersonating ChronoAI, its employees, or other Users;
- engaging in any activity that disrupts, degrades, or impairs the performance of the App or ChronoAI's systems;
- using the App to facilitate money laundering, financing of terrorism, or any other financial crime;
- sharing, distributing, or publishing any NyxID Content for commercial purposes without ChronoAI's prior written consent;
- engaging in any other activity that ChronoAI, in its reasonable discretion, determines to be harmful, illegal, or inconsistent with these Terms.

### 5.6 Responsibility for Violations

You acknowledge that you are solely responsible for any violation of applicable laws or these Terms arising from your use of the App. You agree to indemnify, defend, and hold harmless ChronoAI and its officers, directors, employees, agents, and licensors from and against any and all claims, liabilities, damages, losses, costs, and expenses (including reasonable legal fees) arising out of or related to your violation of any applicable law or these Terms.

### 5.7 Developer Obligations (SDK and Partner Applications)

If you use the SDK or otherwise build a Partner Application that integrates with the App (for example, via "Sign in with NyxID" or other OAuth/OIDC flows), you additionally agree that:

- you will use OAuth client credentials only for the Partner Application registered to receive them, will not share them with third parties, will not embed client secrets in distributed client-side or mobile binaries, and will rotate them on any suspected compromise;
- you will configure and maintain redirect URIs in good faith, will not register redirect URIs you do not control, and will treat tokens issued by the App as bearer credentials to be stored and transmitted accordingly;
- you will not misrepresent your identity or your Partner Application's affiliation with ChronoAI, and you will use NyxID branding only as expressly permitted by ChronoAI's brand guidelines;
- you will handle End User data obtained through the App in accordance with a publicly accessible privacy policy that satisfies applicable law (including the GDPR and PDPA where relevant); and
- ChronoAI may suspend or revoke developer access at any time for breach of these Terms or where required by applicable law.

## 6. SECURITY DISCLAIMERS AND AUTHENTICATION-SPECIFIC OBLIGATIONS

### 6.1 No Guarantee of Absolute Security

While ChronoAI implements industry-standard security measures designed to protect your credentials and data — including encryption at rest and in transit, multi-factor authentication, role-based access controls, and regular security assessments — no security system is infallible. ChronoAI does not warrant or represent that the App or its underlying infrastructure is invulnerable to security breaches, cyberattacks, or unauthorised access.

ChronoAI's security practices are informed by internationally recognised frameworks, including ISO/IEC 27001 (Information Security Management), the NIST Cybersecurity Framework, and SOC 2 Type II standards. However, references to these frameworks do not constitute a warranty that ChronoAI is formally certified under such frameworks, unless expressly stated.

### 6.2 User Responsibility for Credential Security

You acknowledge and agree that:

- you are solely responsible for the security of the credentials you store within the App;
- ChronoAI's Credential Proxy only injects credentials into requests initiated by you or your authorised agents, and any misuse resulting from compromised credentials on your end is your sole responsibility;
- the approval workflow (push notification and messaging-platform messages) is a security feature that you are strongly encouraged to enable; ChronoAI accepts no liability for unauthorised proxied requests where you have disabled the approval requirement, except to the extent such loss arises from ChronoAI's gross negligence, wilful misconduct, or failure to implement industry-standard security measures, or where liability cannot be excluded under applicable law;
- SSH Certificates issued by the App are short-lived and should be monitored; you are responsible for revoking certificates where there is a suspected compromise;
- pairing codes issued by the App for CLI remote pairing (and any similar short-lived shared codes used to bind a local agent or session to your account) are sensitive credentials; you must treat them as such and not share them with any party who is not authorised to act on your behalf.

### 6.3 Limitation of Liability for Security Incidents

ChronoAI shall not be liable for any loss, damage, or liability arising from:

- your disclosure of account credentials or API Keys and Tokens to unauthorised third parties;
- the compromise of your device or local environment, including keyloggers, malware, or physical access by unauthorised persons;
- the actions of third-party services to which credentials are proxied;
- the failure of third-party OAuth providers (including Google, GitHub, and Apple) whose security posture is outside ChronoAI's control;
- force majeure events, including cyberattacks of exceptional scale or sophistication that could not reasonably have been anticipated or mitigated.

### 6.4 Biometric and Device Fingerprinting Data

If the App utilises biometric data (such as fingerprint or facial recognition) for device-level authentication, such biometric processing is performed by your device's operating system and is not transmitted to or stored by ChronoAI. ChronoAI does not store biometric templates. Device fingerprinting data (including device identifiers, user-agent strings, and similar metadata) is collected as session metadata for security and fraud prevention purposes in accordance with the Privacy Policy.

### 6.5 Incident Reporting

If you become aware of any actual or suspected security incident, data breach, or unauthorised use of your account or credentials, you must notify ChronoAI immediately at **contact@chrono-ai.fun**. Your cooperation in incident response is essential to minimising potential harm.

## 7. DATA PROTECTION AND PRIVACY

### 7.1 Data Collection and Processing

ChronoAI collects and processes personal data in connection with your use of the App, as described in detail in the Privacy Policy. Your use of the App constitutes your acknowledgement of and agreement to ChronoAI's data practices as set out in the Privacy Policy.

### 7.2 PDPA Compliance (Singapore)

ChronoAI is subject to the PDPA and is committed to complying with all applicable obligations thereunder. In particular, ChronoAI will: (i) obtain valid consent, or rely on another lawful basis available under the PDPA (such as deemed consent, the legitimate interests exception, or legal/business improvement purposes), before collecting, using, or disclosing your personal data; (ii) notify you of the purposes for which your data is collected; (iii) implement reasonable security arrangements to protect your personal data; (iv) retain personal data only for as long as necessary for the stated purposes; and (v) apply appropriate safeguards to cross-border transfers of personal data.

### 7.3 GDPR Compliance (EU/EEA Users)

Where the GDPR applies to ChronoAI's processing of your personal data (including where you are an EU or EEA resident), ChronoAI will comply with all applicable GDPR obligations, including: (i) processing your data on a lawful basis; (ii) honouring your rights of access, rectification, erasure, portability, restriction, and objection; (iii) conducting Data Protection Impact Assessments for high-risk processing activities; and (iv) notifying the relevant supervisory authority within 72 hours of becoming aware of a personal data breach where required under Article 33, and notifying affected individuals without undue delay where the breach is likely to result in a high risk to their rights and freedoms under Article 34.

### 7.4 Cross-Border Data Transfers

Personal data collected through the App may be transferred to, stored in, and processed in countries outside your jurisdiction, including Singapore and other countries where ChronoAI's service providers operate. Where personal data is transferred from the EEA, the UK, or other transfer-restricted jurisdictions, ChronoAI will implement appropriate safeguards, including Standard Contractual Clauses approved by the European Commission, adequacy decisions, or other legally recognised transfer mechanisms. Details of data storage locations and transfer safeguards are set out in the Privacy Policy.

### 7.5 Authentication Data as Sensitive Security Data

ChronoAI recognises that API Keys and Tokens, MFA Secrets, and other authentication credentials stored within the App constitute highly sensitive security data. ChronoAI applies heightened security measures to such data, including strong encryption (at rest using AES-256 or equivalent, and in transit using TLS 1.2 or higher), strict access controls, and audit logging of all access events. Notwithstanding the above, ChronoAI cannot guarantee absolute security with respect to this data.

## 8. THIRD-PARTY INTEGRATIONS AND SERVICES

### 8.1 Third-Party Services Generally

The App integrates with, and enables the proxying of credentials to, third-party services selected by you ("**Third-Party Services**"). ChronoAI does not control, endorse, or assume responsibility for the security, reliability, availability, or data practices of any Third-Party Services. Your use of any Third-Party Service is governed solely by that service's own terms and conditions and privacy policy.

### 8.2 OAuth Providers and Social Login

The App supports authentication via third-party OAuth providers including Google, GitHub, and Apple (collectively, "**OAuth Providers**"). When you use social login, we receive limited profile information (name, email address, and provider-specific user identifier) from the relevant OAuth Provider. ChronoAI is not responsible for the security or privacy practices of OAuth Providers.

### 8.3 Messaging-Platform Integrations and Channel Bots

The App supports integration with third-party messaging and collaboration platforms (for example, Telegram, Lark / Feishu, Discord, or similar) in two modes:

- **Outbound notifications.** Linking a messaging-platform account to the App for the purposes of receiving Approval Requests or other notifications. In this mode, ChronoAI collects and processes the minimum identifiers required for the integration (for example, your platform user ID, chat ID, and display name).
- **Inbound Channel Bot routing.** Registering a Channel Bot you operate on a platform (for example, a Telegram bot, a Lark / Feishu bot, or a Discord bot) with the App so that inbound messages addressed to that bot are routed to agents or other services you have authorised. In this mode, you supply ChronoAI with the bot credentials and verification material required by the platform (which may include bot tokens, app IDs and secrets, verification tokens, and encrypt keys). You are responsible for:

  - the lawful registration and continued compliance of your Channel Bot with the operator's developer terms, bot policies, and end-user requirements;
  - the custody of bot credentials supplied to ChronoAI, and for prompt rotation of those credentials upon any suspected compromise;
  - the rotation of webhook signing secrets you generate; and
  - the use of Reply Tokens (single-use tokens ChronoAI issues to authorise per-message replies) only by the agent runtime intended to consume them.

The App may also expose an HTTP event-ingress endpoint allowing devices or other systems you operate to deliver structured events into a conversation flow. The same custody, rotation, and platform-compliance obligations described above apply to credentials and material supplied for such event ingress.

ChronoAI is not affiliated with any messaging-platform operator and is not responsible for their privacy or security practices. Your use of each such platform is governed by the terms of service and privacy policy of that platform's operator.

### 8.4 Cloud Infrastructure and Hosting Providers

The App is hosted on cloud infrastructure provided by third-party providers. All such providers are engaged pursuant to data processing agreements that require them to implement appropriate security measures and to process data only on ChronoAI's instructions. Details of primary cloud infrastructure providers and data storage regions are set out in the Privacy Policy.

### 8.5 Analytics and Communications Providers

ChronoAI may use third-party analytics, marketing, and communications platforms in connection with the Services. For example, waitlist sign-up data (first name, email, optional company name) may be transmitted to third-party mailing list providers (such as Mailchimp) for communications purposes, and is not stored persistently by NyxID. You will be informed of, and your consent sought for, any use of third-party analytics or communications tools that involve the processing of your personal data.

### 8.6 Apple App Store and Google Play Requirements

The App is distributed through the Apple App Store and the Google Play Store (collectively, "**App Platforms**"). ChronoAI complies with the App Platform guidelines and requirements applicable to the App, including Apple's App Tracking Transparency ("**ATT**") framework. Where required under ATT or equivalent requirements, ChronoAI will seek your explicit consent before engaging in cross-app tracking. ChronoAI's App complies with applicable privacy label requirements for the disclosure of data categories collected.

The App Platforms are not parties to this Agreement. ChronoAI is responsible for the App and the content thereof. The App Platforms disclaim all warranties in respect of the App and have no obligation to furnish any maintenance or support services with respect thereto.

## 9. INTELLECTUAL PROPERTY RIGHTS

### 9.1 Ownership

The App, its underlying software, architecture, and all content, trademarks, logos, design, text, and other proprietary materials available through the Services (collectively, "**NyxID Content**") are owned by ChronoAI Pte. Ltd. or licensed to ChronoAI by third-party licensors. All intellectual property rights in the NyxID Content are reserved. Nothing in these Terms shall be construed as transferring any intellectual property rights to you.

### 9.2 Limited Licence

Subject to your compliance with these Terms, ChronoAI grants you a limited, non-exclusive, non-sublicensable, non-transferable, revocable licence to access and use the App and NyxID Content for your personal or internal business use as permitted by these Terms.

### 9.3 Restrictions

You agree that you will not, and will not permit any third party to:

- reproduce, modify, adapt, translate, distribute, publicly display, sell, lease, reverse engineer, decompile, or disassemble the App or any NyxID Content;
- use any NyxID Content for purposes outside the licence granted in Section 9.2 without ChronoAI's prior written consent;
- attempt to circumvent, disable, or interfere with any security or access control feature of the App;
- remove or obscure any proprietary rights notices in or accompanying the NyxID Content.

### 9.4 User-Generated Content

To the extent you submit any content, feedback, suggestions, or information to ChronoAI through the App ("**User Content**"), you hereby grant ChronoAI a non-exclusive, royalty-free, worldwide, perpetual licence to use, reproduce, modify, and incorporate such User Content into the Services for the purposes of improving and developing the App. You represent and warrant that you have all rights necessary to grant this licence.

## 10. SERVICE CHANGES, SUSPENSION AND TERMINATION

### 10.1 Service Modifications

ChronoAI may, at its sole discretion, modify, introduce, or discontinue any part of the Services at any time. ChronoAI will use reasonable endeavours to provide advance notice of material modifications, but reserves the right to make changes without prior notice where required by law, regulation, or security considerations.

### 10.2 Temporary Suspension

ChronoAI may temporarily suspend the Services in the following circumstances:

- scheduled or emergency maintenance, system upgrades, or infrastructure work;
- force majeure events, including natural disasters, acts of war, terrorist attacks, cyberattacks of exceptional scale, power outages, or government orders;
- failures of third-party infrastructure or telecommunications networks beyond ChronoAI's reasonable control; or
- security incidents requiring immediate investigation or remediation.

### 10.3 Unilateral Suspension or Termination

ChronoAI reserves the right to unilaterally suspend or terminate your access to all or any part of the Services, with or without notice, for any of the following reasons:

- your breach of any provision of these Terms;
- use of the App for illegal or criminal activities;
- suspected or confirmed compromise of your account that poses a risk to other Users or ChronoAI's systems;
- your failure to pay applicable service fees;
- death of the User (upon notification by a verified next of kin or legal representative);
- regulatory, legal, or compliance requirements that necessitate restriction of access; or
- any other circumstance where ChronoAI, in its reasonable discretion, determines that suspension or termination is necessary to protect the integrity, security, or operation of the Services.

### 10.4 Effect of Termination

Upon termination of your access to the Services:

- all licences granted to you under these Terms immediately terminate;
- you must immediately cease all use of the App and delete all copies of the App from your devices;
- ChronoAI will handle your personal data following termination in accordance with the Privacy Policy and applicable law; and
- provisions of these Terms that by their nature should survive termination (including Sections 5, 6, 9, 11, 12, 13, 14, and 15) shall survive.

## 11. INCIDENT AND BREACH HANDLING

### 11.1 Incident Response Commitment

ChronoAI maintains documented incident response procedures designed to detect, contain, investigate, and remediate security incidents in a timely manner. ChronoAI's incident response capabilities are informed by industry best practices, including the NIST Cybersecurity Framework and ISO/IEC 27001 standards.

### 11.2 Data Breach Notification

In the event of a personal data breach that is likely to result in a risk to the rights and freedoms of affected Users, ChronoAI will:

- notify the relevant data protection authority within the timeframes required by applicable law (within 72 hours of becoming aware under the GDPR; no later than 3 calendar days of assessing the breach as notifiable under the Singapore PDPA);
- notify affected Users without undue delay where the breach is likely to result in a high risk to their rights and freedoms; and
- provide affected Users and authorities with information regarding the nature of the breach, categories and approximate number of data subjects affected, likely consequences, and measures taken or proposed.

### 11.3 User Notification Obligations

You agree to notify ChronoAI immediately upon becoming aware of any actual or suspected security breach, unauthorised access to your account, or loss or theft of credentials stored within the App. Timely notification enables ChronoAI to take appropriate protective measures on your behalf.

### 11.4 Cybersecurity Act and NIS2

ChronoAI is aware of and committed to compliance with applicable cybersecurity laws, including the Singapore Cybersecurity Act 2018 and, to the extent applicable to its EU operations, the NIS2 Directive (Directive (EU) 2022/2555). ChronoAI will notify relevant authorities of significant cybersecurity incidents as required by applicable law.

## 12. DISCLAIMER AND LIMITATION OF LIABILITY

> READ THIS SECTION CAREFULLY. IT SIGNIFICANTLY LIMITS CHRONOAI'S LIABILITY TO YOU. BY USING THE APP, YOU ACKNOWLEDGE AND AGREE TO THESE LIMITATIONS.

### 12.1 "As Is" Disclaimer

THE APP AND SERVICES ARE PROVIDED "AS IS", "AS AVAILABLE", AND "WITH ALL FAULTS". TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW, CHRONOAI EXPRESSLY DISCLAIMS ALL WARRANTIES OF ANY KIND, WHETHER EXPRESS, IMPLIED, STATUTORY, OR OTHERWISE, INCLUDING WITHOUT LIMITATION WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE, TITLE, AND NON-INFRINGEMENT. CHRONOAI DOES NOT WARRANT THAT: (I) THE SERVICES WILL MEET YOUR REQUIREMENTS; (II) THE SERVICES WILL BE UNINTERRUPTED, TIMELY, SECURE, OR ERROR-FREE; OR (III) ANY DEFECTS IN THE SERVICES WILL BE CORRECTED.

### 12.2 Exclusion of Consequential Damages

TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW, IN NO EVENT SHALL CHRONOAI, ITS OFFICERS, DIRECTORS, EMPLOYEES, AGENTS, OR LICENSORS BE LIABLE FOR ANY INDIRECT, INCIDENTAL, SPECIAL, CONSEQUENTIAL, PUNITIVE, OR EXEMPLARY DAMAGES, INCLUDING WITHOUT LIMITATION LOSS OF PROFITS, LOSS OF REVENUE, LOSS OF DATA, LOSS OF GOODWILL, BUSINESS INTERRUPTION, OR COSTS OF SUBSTITUTE SERVICES, ARISING OUT OF OR IN CONNECTION WITH YOUR USE OF OR INABILITY TO USE THE APP OR SERVICES, REGARDLESS OF THE CAUSE OF ACTION AND WHETHER BASED IN CONTRACT, TORT, NEGLIGENCE, STRICT LIABILITY, OR OTHERWISE, EVEN IF CHRONOAI HAS BEEN ADVISED OF THE POSSIBILITY OF SUCH DAMAGES.

### 12.3 Specific Liability Exclusions

Without prejudice to the generality of Section 12.2, ChronoAI shall not be liable for:

- loss or damage arising from your disclosure of account credentials or API Keys and Tokens to unauthorised parties;
- loss or damage arising from the compromise of your device, operating environment, or local network;
- loss or damage arising from the actions, failures, or security practices of any Third-Party Service to which credentials are proxied;
- loss or damage arising from your failure to enable, or your disabling of, the Approval Request workflow;
- loss or damage caused by force majeure events or circumstances outside ChronoAI's reasonable control;
- loss or damage resulting from scheduled maintenance, system upgrades, or temporary service interruptions;
- any loss, harm, or disruption arising from the outputs, errors, omissions, or actions of any AI agent that you operate, or that operates on your behalf, in connection with the App.

### 12.4 Aggregate Liability Cap

TO THE MAXIMUM EXTENT PERMITTED BY APPLICABLE LAW, CHRONOAI'S TOTAL AGGREGATE LIABILITY TO YOU FOR ALL CLAIMS ARISING OUT OF OR IN CONNECTION WITH THIS AGREEMENT, REGARDLESS OF THE FORM OF THE ACTION, SHALL NOT EXCEED THE GREATER OF: (I) THE TOTAL FEES PAID BY YOU TO CHRONOAI IN THE TWELVE (12) MONTHS IMMEDIATELY PRECEDING THE EVENT GIVING RISE TO THE CLAIM; OR (II) ONE HUNDRED UNITED STATES DOLLARS (USD 100).

### 12.5 Essential Basis of the Agreement

YOU ACKNOWLEDGE THAT THE DISCLAIMERS AND LIMITATIONS OF LIABILITY IN THIS SECTION 12 REFLECT A REASONABLE AND FAIR ALLOCATION OF RISK BETWEEN THE PARTIES, AND THAT CHRONOAI WOULD NOT HAVE ENTERED INTO THIS AGREEMENT WITHOUT THESE LIMITATIONS. THESE LIMITATIONS SHALL APPLY NOTWITHSTANDING ANY FAILURE OF ESSENTIAL PURPOSE OF ANY LIMITED REMEDY. NOTHING IN THESE TERMS SHALL EXCLUDE OR LIMIT ANY LIABILITY THAT CANNOT LAWFULLY BE EXCLUDED OR LIMITED UNDER APPLICABLE LAW (INCLUDING LIABILITY FOR DEATH OR PERSONAL INJURY CAUSED BY NEGLIGENCE, OR FOR FRAUD OR FRAUDULENT MISREPRESENTATION).

### 12.6 No Professional Advice

ChronoAI does not provide legal, tax, investment, medical, psychological, or other professional advice. Nothing in the App or the Services constitutes professional advice of any kind. Users should seek independent professional advice for any such matters.

## 13. YOUR REPRESENTATIONS AND WARRANTIES

By accessing or using the App, you represent and warrant to ChronoAI that:

- you have the legal capacity and authority to enter into this Agreement;
- all information you provide to ChronoAI is accurate, complete, and current;
- you will use the App only for lawful purposes and in accordance with these Terms;
- you are not subject to any sanctions or export control restrictions that would prohibit your use of the Services;
- you will comply with all applicable laws, regulations, and third-party terms of service in connection with your use of the App;
- any credentials you store within the App are yours to use and are not subject to any restrictions that would prohibit their use in connection with the Credential Proxy functionality; and
- you will not use the App to circumvent the security controls, access controls, or terms of service of any third-party system or service.

## 14. FEES AND PAYMENT

### 14.1 Applicable Fees

As at the Effective Date of these Terms, ChronoAI does not charge fees for use of the App. ChronoAI reserves the right to introduce fees ("**Service Fees**") for access to all or part of the Services in the future, in which case such fees will be disclosed to you prior to your incurring them. Service Fees may include subscription fees, per-use charges, or other pricing structures as notified by ChronoAI from time to time.

### 14.2 Payment Obligations

You agree to pay all applicable Service Fees in a timely manner. ChronoAI reserves the right to suspend or terminate your access to the Services if you fail to pay any fees when due. All in-app purchases and payment processing are subject to the terms and conditions of the applicable App Platform's payment system (e.g., Apple In-App Purchase or Google Play Billing). ChronoAI is not responsible for payment processing errors, disputes, or refunds originating from App Platform payment systems.

### 14.3 Changes to Fees

ChronoAI reserves the right to modify its pricing and Service Fees at any time, with reasonable prior notice to Users.

## 15. BINDING ARBITRATION AND CLASS ACTION WAIVER

> PLEASE READ THIS SECTION CAREFULLY — IT MAY SIGNIFICANTLY AFFECT YOUR LEGAL RIGHTS, INCLUDING YOUR RIGHT TO FILE A LAWSUIT IN COURT AND YOUR ABILITY TO BRING A CLASS ACTION.

### 15.1 Binding Arbitration

Any dispute, claim, or controversy ("**Claim**") relating in any way to this Agreement or your use of the App will, to the maximum extent permitted by applicable law, be resolved by binding arbitration rather than in court, except that you may assert claims in small claims court if your claims qualify.

### 15.2 Governing Law and Forum (Singapore — Sole Jurisdiction)

This Agreement and any Claim (including non-contractual disputes or claims) arising out of or in connection with it, or its subject matter or formation, shall be governed by and construed in accordance with the laws of Singapore, regardless of where you reside or where you access the App. Any Claim shall be submitted first to mediation in accordance with the Singapore International Arbitration Centre ("**SIAC**") Mediation Rules, which are incorporated by reference. If the dispute is not settled by mediation within fourteen (14) days of commencement, it shall be referred to and finally resolved by arbitration under the SIAC Rules. The arbitration tribunal shall consist of a single arbitrator, appointed by agreement of the parties, or failing agreement, by the President of the Court of Arbitration of SIAC. The seat of arbitration shall be Singapore. The language shall be English.

To the extent that mandatory consumer-protection or data-protection laws of your country of residence cannot be lawfully waived by contract, nothing in this Section 15.2 prevents you from invoking those mandatory protections.

### 15.3 Class Action Waiver

YOU AND CHRONOAI AGREE THAT EACH PARTY MAY BRING CLAIMS AGAINST THE OTHER ONLY ON AN INDIVIDUAL BASIS, AND NOT AS A PLAINTIFF OR CLASS MEMBER IN ANY PURPORTED CLASS, COLLECTIVE, OR REPRESENTATIVE PROCEEDING. THE PARTIES EXPRESSLY WAIVE ANY RIGHT TO FILE A CLASS ACTION OR SEEK RELIEF ON A CLASS BASIS. If a court of competent jurisdiction determines that this class action waiver is void or unenforceable as to a particular claim, the arbitration provisions shall not apply to that claim, and it must be brought in a court of competent jurisdiction.

## 16. MISCELLANEOUS

### 16.1 Assignment

You may not assign or transfer this Agreement or any of your rights or obligations hereunder without ChronoAI's prior written consent. ChronoAI may assign this Agreement without your consent in connection with a merger, acquisition, sale of all or substantially all of its assets, or corporate reorganisation. Any purported assignment in violation of this Section shall be void.

### 16.2 Entire Agreement

This Agreement, together with the Privacy Policy and any other documents incorporated by reference, sets forth the entire understanding and agreement between you and ChronoAI with respect to the subject matter hereof and supersedes all prior discussions, agreements, and understandings of any kind.

### 16.3 Severability

If any provision of this Agreement is found by a court or arbitrator of competent jurisdiction to be invalid, illegal, or unenforceable, such provision shall be modified to the minimum extent necessary to make it enforceable, or if not possible, severed from this Agreement. The remaining provisions shall continue in full force and effect.

### 16.4 Independent Contractors

The relationship between you and ChronoAI is that of independent contractors. Nothing in these Terms shall be construed as creating a partnership, joint venture, agency, fiduciary, or employment relationship between the parties.

### 16.5 Waiver

No failure or delay by ChronoAI in exercising any right or remedy under this Agreement shall constitute a waiver of that right or remedy. A waiver by ChronoAI of any breach or default shall not constitute a waiver of any subsequent breach or default.

### 16.6 Force Majeure

ChronoAI shall not be in breach of this Agreement or liable for any delay or failure in performance resulting from causes beyond its reasonable control, including acts of God, natural disasters, war, terrorism, civil unrest, government orders, power failures, internet service interruptions, or cyberattacks of exceptional scale. ChronoAI will use reasonable endeavours to mitigate the impact of force majeure events and to resume normal operations as soon as practicable.

### 16.7 Notices

Notices or other communications from ChronoAI under these Terms will be provided by posting to the App, by displaying in-app notifications, or by emailing the address associated with your account. You agree to receive electronic communications from ChronoAI relating to your account and use of the Services. Notices from you to ChronoAI must be submitted to **contact@chrono-ai.fun** or to the postal address set out in Section 16.9.

### 16.8 Governing Language

This Agreement is drafted in the English language. In the event of any conflict between the English version and any translated version, the English version shall prevail.

### 16.9 Contact

If you have any questions, concerns, or complaints regarding these Terms, please contact us:

**ChronoAI Pte. Ltd.**
Address: 8 Marina Boulevard, #14-02, Singapore 018981
Contact: **contact@chrono-ai.fun**
Website: **https://nyx.chrono-ai.fun/**

