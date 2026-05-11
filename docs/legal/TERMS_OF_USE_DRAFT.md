# NyxID — Terms of Use

**Status:** Draft — pending Legal sign-off. Tracks Issue #499 item 2.
**Last updated:** 11 May 2026

> **IMPORTANT NOTICE:** PLEASE READ THESE TERMS OF USE CAREFULLY BEFORE ACCESSING OR USING THE NYXID APPLICATION. BY ACCESSING OR USING THE APP, YOU CONFIRM THAT YOU HAVE READ, UNDERSTOOD, AND AGREE TO BE LEGALLY BOUND BY THESE TERMS. PAY PARTICULAR ATTENTION TO **SECTIONS 4 (USER-OPERATED AI AGENTS), 6 (SECURITY DISCLAIMERS), 9 (INTELLECTUAL PROPERTY AND USER CONTENT), 10 (SUSPENSION AND TERMINATION), 12 (LIMITATION OF LIABILITY), 14 (FEES, REFUNDS AND AUTO-RENEWAL), AND 15 (ARBITRATION AND CLASS ACTION WAIVER).** IF YOU DO NOT AGREE TO THESE TERMS, DO NOT ACCESS OR USE THE APP.

These NyxID Terms of Use ("**Terms**") constitute a legal agreement between you ("**User**" or "**you**") and ChronoAI Pte. Ltd. ("**we**," "**us**," or "**ChronoAI**") (the "**Agreement**"). These Terms apply when you visit or interact with the NyxID application, engage with our customer support, interact with us on social media, or otherwise communicate with us. By accessing or using the App, you agree to be bound by these Terms.

## 1. CONFIRMATION AND ACCEPTANCE OF THESE TERMS

### 1.1 Entire Agreement and Scope of Applicability

These Terms of Use ("**Terms**"), together with the Privacy Policy and any other documents expressly incorporated by reference herein (collectively, the "**Agreement**"), constitute the entire and exclusive agreement between you ("**User**" or "**you**") and ChronoAI Pte. Ltd. ("**ChronoAI**", "**we**", "**us**", or "**our**") concerning your access to and use of the NyxID mobile application, web application, command-line interface ("CLI"), and node agent software (collectively, the "**App**") and all related services, features, and content provided by ChronoAI (collectively, the "**Services**").

This Agreement supersedes all prior or contemporaneous communications, proposals, or agreements, whether electronic, oral, or written, relating to the subject matter hereof. For the avoidance of doubt, this Agreement does not extend to, nor does ChronoAI assume any responsibility or liability for: (i) third-party services accessed through the Credential Proxy; (ii) third-party large language model providers used by the AI chat assistant; (iii) OAuth Providers used for social login; (iv) Channel Platform operators; (v) Self-Hosted Deployments operated by third parties; or (vi) Partner Applications that integrate with NyxID, each of which is governed by its own terms and policies.

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

- be at least (a) thirteen (13) years of age, (b) at or above the digital-services consent age in your jurisdiction (which may be 16 in some EU member states) or have verifiable parental or guardian consent, and (c) at or above the age of majority in your jurisdiction to enter into a binding contract — or, if you are below the age of majority, use the App only with the consent and supervision of a parent or legal guardian who agrees to be bound by these Terms on your behalf;
- not be a resident of, or located in, a jurisdiction subject to applicable trade embargoes, UN Security Council Resolutions, or sanctions regimes (including those administered by OFAC, HM Treasury, or the UN Sanctions Committee);
- not be listed on any sanctions list, including the UN Security Council Consolidated List, the U.S. Specially Designated Nationals List, or any equivalent list maintained by a relevant authority;
- not use the Services if doing so would violate any applicable law or regulation in your jurisdiction.

If you are accessing the Services on behalf of a legal entity, you represent and warrant that the entity is duly incorporated and that you are duly authorised to act on its behalf and bind it to this Agreement. ChronoAI reserves the right to modify eligibility criteria and restrict access at any time.

### 1.7 Scope — Hosted Service Only

These Terms govern your use of the **NyxID hosted Service operated by ChronoAI**. NyxID may also be deployed and operated by third parties on their own infrastructure ("**Self-Hosted Deployments**"). If you access NyxID via a Self-Hosted Deployment, your use is governed by the operator's own terms and privacy policy. ChronoAI is not a controller in respect of personal data processed by a Self-Hosted Deployment and is not a party to the contractual relationship between the operator and its end users. Operators of Self-Hosted Deployments are themselves Users of the open-source NyxID software and are responsible for their own legal and regulatory compliance toward their end users (see also Section 8.7).

## 2. DEFINITIONS

For the purposes of this Agreement, the following terms shall have the meanings set out below:

- **"API Keys and Tokens"** means authentication credentials, API keys, OAuth tokens, SSH certificates, and other similar access credentials that you store within the App for the purpose of proxied service access.
- **"App"** means the NyxID mobile application, web application, CLI, and node agent software, including all updates, upgrades, and versions thereof.
- **"Approval Request"** means a push notification or messaging-platform message sent to you requiring your approval or denial before a proxied credential request is executed.
- **"ChronoAI" / "we" / "us" / "our"** means ChronoAI Pte. Ltd., a company incorporated in Singapore (Company Registration No.: **[Legal: confirm UEN]**), and its successors and assigns.
- **"Channel Platform"** means any third-party messaging or collaboration platform (including Telegram, Lark/Feishu, Discord, and OpenClaw) integrated with the App for the delivery of Approval Requests, notifications, or chat-based interactions.
- **"Credential Proxy"** means the functionality by which the App injects your stored API Keys and Tokens into outbound requests to third-party services on your behalf.
- **"End-User"** means a natural person who interacts with a Partner Application that integrates with NyxID; an End-User of a Partner Application may or may not also be a User of the NyxID App.
- **"GDPR"** means the General Data Protection Regulation (EU) 2016/679, as amended or replaced from time to time.
- **"Intellectual Property"** means all patents, copyrights, trademarks, trade secrets, database rights, design rights, and all other intellectual property rights, whether registered or unregistered.
- **"LLM"** means a large language model used by the AI Chat Assistant or other AI Features within the App.
- **"Licensed Application"** has the meaning given in Section 8.6 in relation to the NyxID iOS application as obtained from the Apple App Store.
- **"Local Agent"** means software operated by the User on their own hardware — including the NyxID node agent and the `nyxid` CLI — which may store API Keys and Tokens locally without transmitting them to ChronoAI's servers.
- **"MFA Secrets"** means multi-factor authentication seeds, time-based one-time password (TOTP) secrets, and similar data used to generate authentication codes.
- **"Mobile App"** means the NyxID iOS and/or Android application as distributed through the Apple App Store or Google Play Store.
- **"NyxID Content"** has the meaning given in Section 9.1 and includes the App, its underlying software, architecture, AI models and systems, and all related content, trademarks, logos, design, text, and other proprietary materials.
- **"OAuth/OIDC"** means the OAuth 2.0 and OpenID Connect authentication protocols.
- **"Partner Application"** means any application developed or operated by a third party that integrates with the App via "Sign in with NyxID" or similar shared authentication functionality.
- **"PDPA"** means the Personal Data Protection Act 2012 (Singapore), as amended or replaced from time to time, including applicable subsidiary legislation and guidelines issued by the Personal Data Protection Commission (PDPC).
- **"Personal Data"** has the meaning given to it under applicable data protection law, and includes information from which you can be identified directly or indirectly.
- **"SDK"** means the official NyxID software development kits and client libraries (including `@nyxids/oauth-core` and `@nyxids/oauth-react`) made available by ChronoAI for the purpose of integrating Partner Applications with NyxID.
- **"Self-Hosted Deployment"** means an instance of the NyxID software deployed and operated by a third party on infrastructure not controlled by ChronoAI.
- **"Services"** means all features, functions, tools, and content made available through the App, as further described in Section 3.
- **"SSH Certificate"** means a short-lived cryptographically signed certificate issued by the App for the purpose of authenticating remote server access.
- **"User" / "you" / "your"** means any natural person who accesses or uses the App, including both registered account holders and visitors; for the purposes of Section 1.6 (Eligibility), visitors who only browse public-facing pages are subject only to the conditions of access enumerated there.

## 3. SERVICES AND FUNCTIONALITIES

### 3.1 Service Description

NyxID is an identity and secure credential proxy service. The App enables Users to create an account, securely store API Keys and Tokens for remote third-party services, and have those credentials injected into outbound requests via the Credential Proxy functionality. NyxID is network and identity infrastructure; it does not itself incorporate artificial intelligence or machine-learning features. See Section 4 for the position on user-operated AI agents.

### 3.2 Core Functionalities

The App provides the following core Services:

- **Credential Storage and Proxy:** You may store API Keys and Tokens within the App (encrypted at rest). NyxID proxies requests to third-party services by injecting your stored credentials on your behalf. Proxied request and response bodies are buffered in memory for the Approval Request flow only and are not written to disk or persistently logged.
- **Local Agent:** You may optionally run a Local Agent (the NyxID node agent or `nyxid` CLI) on your own hardware, allowing credentials to remain on your device and never be transmitted to ChronoAI's servers.
- **Approval Interface:** The mobile App and supported Channel Platforms serve as approval interfaces, enabling you to approve, deny, or revoke access requests via push notification (iOS/Android) or messaging-platform message before each proxied request is executed.
- **OAuth/OIDC Login Provider:** NyxID can act as an OAuth/OIDC login provider, enabling third-party developers to integrate "Sign in with NyxID" into their applications.
- **SSH Certificate Issuance:** The App can issue short-lived SSH Certificates for authenticating remote server access.

### 3.3 Service Evolution

ChronoAI reserves the right to introduce, modify, suspend, or discontinue any Service or feature at any time. Where changes materially affect your use of the Services, ChronoAI will use reasonable endeavours to provide prior notice. Your continued use of the App following any change constitutes your acceptance of the modified Services.

### 3.4 Developer Integrations and SDK Use

If you are a developer integrating a Partner Application with NyxID (including via "Sign in with NyxID," OAuth client credentials, or the SDK), you additionally agree that:

- you will not misrepresent your identity or your application's affiliation with ChronoAI, and you will not use NyxID branding except as expressly permitted by ChronoAI's brand guidelines;
- you will use OAuth client credentials only for the application registered to receive them, will not share them with third parties, will rotate them upon any suspected compromise, and will not embed client secrets in distributed client-side or mobile binaries;
- you will collect, use, and disclose End-User data obtained through NyxID in accordance with a publicly accessible privacy policy that satisfies applicable law (including the GDPR and PDPA where relevant); Partner Applications act as independent data controllers of End-User data they collect through NyxID, except where the parties have entered into a separate data processing agreement under GDPR Article 28 or equivalent;
- you will not use confidential or non-publicly-available API specifications, schemas, or pre-release SDK builds, nor bulk-export, mirror, or republish data obtained through NyxID, to develop a service competing with NyxID;
- you will indemnify ChronoAI for End-User claims arising from your Partner Application's own collection, use, or disclosure of End-User data;
- ChronoAI may inspect aggregate API usage records for the purpose of detecting abuse, fraud, or breach of these Terms, subject to applicable privacy law;
- ChronoAI may revoke developer access at any time for breach of these Terms or where required by applicable law.

## 4. USER-OPERATED AI AGENTS ("BYOK")

### 4.1 No Inherent AI in the App

NyxID does not itself incorporate artificial intelligence or machine-learning features. The App is identity and credential-broker infrastructure. Any artificial intelligence used in connection with NyxID is supplied and operated by you ("Bring Your Own Key" / BYOK).

### 4.2 User-Operated AI Agents

You may use third-party AI agents (for example, Claude Code, Codex, OpenClaw, or similar) to interact with the App. Such AI agents act under your authority and using credentials you supply, including scoped API keys issued through your NyxID account. ChronoAI does not control, supervise, or assume responsibility for AI agents operated by you or on your behalf.

### 4.3 Responsibility for AI Agent Actions

You are solely responsible for all actions performed by any AI agent acting under your account or API key, including the agent's compliance with these Terms and with the terms of any Third-Party Services accessed via the Credential Proxy. ChronoAI accepts no liability for the outputs, errors, omissions, or unauthorised actions of AI agents operated by you or on your behalf, to the maximum extent permitted by applicable law.

### 4.4 No Automated Decision-Making by ChronoAI with Legal Effect

ChronoAI does not use the App to make automated decisions that produce legal effects concerning you or similarly significantly affect you. Where the App incorporates automated controls (for example, rate limiting, abuse detection, or session termination on suspected compromise), those controls are deterministic security features applied uniformly to all Users, not AI-driven assessments of you as an individual. If you believe an automated control has affected you in error, you may contact ChronoAI at **contact@chrono-ai.fun**.

## 5. USER RIGHTS AND OBLIGATIONS

### 5.1 User Rights

Subject to these Terms, ChronoAI grants you the following rights with respect to your data and account:

- to access, correct, or request deletion of your personal data at any time in accordance with the Privacy Policy;
- to revoke any OAuth consent, approval grant, or API key at any time via the App;
- to choose where credentials are stored — on ChronoAI's servers or on your own hardware via the Local Agent;
- to disconnect social logins, Channel Platform integrations, and push notification services at any time;
- to export your data in a portable format where technically feasible.

### 5.2 Device Security Obligations

You are solely responsible for maintaining the security of your device(s) used to access the App. You agree to:

- use device-level security measures appropriate to the sensitivity of credentials stored on the device (for example, screen lock, biometric authentication, or device encryption, where supported by your device);
- keep your device's operating system and the App updated to the latest version;
- immediately notify ChronoAI at **contact@chrono-ai.fun** if you suspect your device has been lost, stolen, or compromised;
- not jailbreak, root, or otherwise modify your device in a manner that circumvents security controls; use of the App on a jailbroken or rooted device may void ChronoAI's security warranties to the maximum extent permitted by law and may be grounds for suspension or termination under Section 10.3;
- not install or permit the installation of software that may intercept, monitor, or tamper with App communications or stored credentials.

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
- attempting to reverse engineer, decompile, disassemble, or otherwise derive the source code of the App or its AI systems, except to the extent that such activity is expressly permitted by applicable open-source licence terms governing components of the App;
- uploading, transmitting, or storing malware, viruses, or other malicious code through the App;
- conducting penetration testing, vulnerability scanning, or any security testing of the App or ChronoAI's infrastructure without prior written authorisation;
- impersonating ChronoAI, its employees, or other Users;
- engaging in any activity that disrupts, degrades, or impairs the performance of the App or ChronoAI's systems;
- using the App to facilitate money laundering, financing of terrorism, or any other financial crime;
- sharing, distributing, or publishing any NyxID Content for commercial purposes other than as expressly permitted by these Terms or by ChronoAI's prior written consent;
- engaging in any other activity that ChronoAI, in its reasonable discretion, determines to be harmful, illegal, or inconsistent with these Terms.

### 5.6 Responsibility for Violations

You acknowledge that you are solely responsible for any violation of applicable laws or these Terms arising from your use of the App. You agree to indemnify, defend, and hold harmless ChronoAI and its officers, directors, employees, agents, and licensors from and against any and all claims, liabilities, damages, losses, costs, and expenses (including reasonable legal fees) arising out of or related to your violation of any applicable law or these Terms.

## 6. SECURITY DISCLAIMERS AND AUTHENTICATION-SPECIFIC OBLIGATIONS

### 6.1 No Guarantee of Absolute Security

While ChronoAI implements industry-standard security measures designed to protect your credentials and data — including encryption at rest and in transit, multi-factor authentication, role-based access controls, and regular security assessments — no security system is infallible. ChronoAI does not warrant or represent that the App or its underlying infrastructure is invulnerable to security breaches, cyberattacks, or unauthorised access.

ChronoAI's security practices are informed by internationally recognised frameworks, including ISO/IEC 27001 (Information Security Management), the NIST Cybersecurity Framework, and SOC 2 Type II standards. References to these frameworks do not constitute a warranty that ChronoAI is formally certified under such frameworks, unless expressly stated.

### 6.2 User Responsibility for Credential Security

You acknowledge and agree that:

- you are solely responsible for the security of the credentials you store within the App;
- ChronoAI's Credential Proxy only injects credentials into requests initiated by you or your authorised agents, and any misuse resulting from compromised credentials on your end is your sole responsibility;
- the approval workflow (push notification and Channel Platform messages) is a security feature that you are strongly encouraged to enable; ChronoAI accepts no liability for unauthorised proxied requests where you have disabled the approval requirement, except to the extent such loss arises from ChronoAI's gross negligence, wilful misconduct, or failure to implement industry-standard security measures, or where liability cannot be excluded under applicable law;
- SSH Certificates issued by the App are short-lived and should be monitored; you are responsible for revoking certificates where there is a suspected compromise.

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

Personal data collected through the App may be transferred to, stored in, and processed in countries outside your jurisdiction, including Singapore and other countries where ChronoAI's service providers operate. Where personal data is transferred from the EEA, the UK, or other transfer-restricted jurisdictions, ChronoAI will implement appropriate safeguards, including Standard Contractual Clauses approved by the European Commission, adequacy decisions, or other legally recognised transfer mechanisms. Details of data storage locations, transfer safeguards, and the current list of sub-processors are set out in the Privacy Policy.

### 7.5 Authentication Data as Sensitive Security Data

ChronoAI recognises that API Keys and Tokens, MFA Secrets, and other authentication credentials stored within the App constitute highly sensitive security data. ChronoAI applies heightened security measures to such data, including strong encryption (at rest using AES-256 or equivalent, and in transit using TLS 1.2 or higher), strict access controls, and audit logging of all access events. Notwithstanding the above, ChronoAI cannot guarantee absolute security with respect to this data.

## 8. THIRD-PARTY INTEGRATIONS AND SERVICES

### 8.1 Third-Party Services Generally

The App integrates with, and enables the proxying of credentials to, third-party services selected by you ("**Third-Party Services**"). ChronoAI does not control, endorse, or assume responsibility for the security, reliability, availability, or data practices of any Third-Party Services. Your use of any Third-Party Service is governed solely by that service's own terms and conditions and privacy policy.

### 8.2 OAuth Providers and Social Login

The App supports authentication via third-party OAuth providers including Google, GitHub, and Apple (collectively, "**OAuth Providers**"). When you use social login, we receive limited profile information (name, email address, and provider-specific user identifier) from the relevant OAuth Provider. ChronoAI is not responsible for the security or privacy practices of OAuth Providers.

### 8.3 Channel Platform Integrations

If you link a Channel Platform account (including Telegram, Lark/Feishu, Discord, OpenClaw, or any other supported messaging or collaboration platform) to the App for the purposes of receiving Approval Requests or interacting with NyxID via that platform, ChronoAI will collect and process the minimum identifiers required for the integration (for example, your platform user ID, chat ID, and display name). ChronoAI is not affiliated with these Channel Platform operators and is not responsible for their privacy or security practices. Your use of each Channel Platform is governed by the terms of service and privacy policy of that platform's operator. The list of supported Channel Platforms may change from time to time as the App evolves.

### 8.4 Cloud Infrastructure and Hosting Providers

The App is hosted on cloud infrastructure provided by third-party providers. All such providers are engaged pursuant to data processing agreements that require them to implement appropriate security measures and to process data only on ChronoAI's instructions. Details of primary cloud infrastructure providers and data storage regions are set out in the Privacy Policy.

### 8.5 Analytics and Communications Providers

ChronoAI may use third-party analytics, marketing, and communications platforms in connection with the Services. For example, waitlist sign-up data (first name, email, optional company name) may be transmitted to third-party mailing list providers (such as Mailchimp) for communications purposes, and is not stored persistently by NyxID. Product usage telemetry is collected only on an opt-in basis and is processed by a third-party analytics provider (PostHog, US region) as described in the Privacy Policy. You will be informed of, and your consent sought for, any use of third-party analytics or communications tools that involve the processing of your personal data.

### 8.6 Apple App Store and Google Play Requirements

The Mobile App is distributed through the Apple App Store and the Google Play Store (collectively, "**App Platforms**"). ChronoAI complies with the App Platform guidelines and requirements applicable to the App, including Apple's App Tracking Transparency ("**ATT**") framework. Where required under ATT or equivalent requirements, ChronoAI will seek your explicit consent before engaging in cross-app tracking. ChronoAI's App complies with applicable privacy label requirements for the disclosure of data categories collected.

The following terms apply specifically to your use of the NyxID iOS application (the "**Licensed Application**" for purposes of this Section 8.6) as obtained from the Apple App Store:

- **Acknowledgement.** You acknowledge that this Agreement is concluded between you and ChronoAI only, and not with Apple Inc. ("**Apple**"). ChronoAI, not Apple, is solely responsible for the Licensed Application and its content.
- **Scope of Licence.** The licence granted to you for the Licensed Application is a limited, non-transferable licence to use the Licensed Application on any Apple-branded device that you own or control and as permitted by the Usage Rules set forth in the Apple Media Services Terms and Conditions.
- **Maintenance and Support.** ChronoAI is solely responsible for providing any maintenance and support services with respect to the Licensed Application. Apple has no obligation whatsoever to furnish any maintenance or support services in connection with the Licensed Application.
- **Warranty.** ChronoAI is solely responsible for any product warranties, whether express or implied by law, to the extent not effectively disclaimed. In the event of any failure of the Licensed Application to conform to any applicable warranty, you may notify Apple, and Apple will refund the purchase price (if any) for the Licensed Application to you. To the maximum extent permitted by applicable law, Apple will have no other warranty obligation whatsoever with respect to the Licensed Application.
- **Product Claims.** ChronoAI, not Apple, is responsible for addressing any of your or any third party's claims relating to the Licensed Application, including (i) product liability claims; (ii) any claim that the Licensed Application fails to conform to any applicable legal or regulatory requirement; and (iii) claims arising under consumer protection, privacy, or similar legislation, including in connection with the Licensed Application's use of HealthKit and HomeKit (where applicable).
- **Intellectual Property Rights.** In the event of any third-party claim that the Licensed Application or your possession and use of the Licensed Application infringes that third party's intellectual property rights, ChronoAI, not Apple, will be solely responsible for the investigation, defence, settlement, and discharge of any such claim.
- **Legal Compliance.** You represent and warrant that (i) you are not located in a country subject to a U.S. Government embargo or designated as a "terrorist supporting" country; and (ii) you are not listed on any U.S. Government list of prohibited or restricted parties.
- **External Services.** The Licensed Application may enable access to third-party services and data ("**External Services**"). You agree to use External Services at your sole risk, and ChronoAI shall not be responsible for examining or evaluating the content or accuracy of any External Services. This Agreement does not apply to any third-party materials accessed by the Licensed Application.
- **Developer Name and Address.** Inquiries relating to the Licensed Application may be addressed to ChronoAI at the contact address set out in Section 16.9.
- **Third-Party Beneficiary.** You acknowledge and agree that Apple, and Apple's subsidiaries, are third-party beneficiaries of this Agreement, and that, upon your acceptance of this Agreement, Apple will have the right (and will be deemed to have accepted the right) to enforce this Agreement against you as a third-party beneficiary thereof.

If you obtain the NyxID Android application from the Google Play Store, the following terms apply specifically:

- **Acknowledgement.** ChronoAI, not Google LLC ("**Google**"), is solely responsible for the Mobile App and its content.
- **No Party Status.** Google is not a party to this Agreement.
- **Payments and Refunds.** Refunds, billing disputes, and subscription management for App Platform purchases are governed by Google Play's terms and policies.
- **Developer Policy Compliance.** ChronoAI complies with the Google Play Developer Distribution Agreement and the Google Play Developer Program Policies applicable to the Mobile App.

The App Platforms are not parties to this Agreement.

### 8.7 Self-Hosted Deployments

NyxID may be deployed by third-party operators on their own infrastructure under the open-source license accompanying the relevant components of the App. Where you access NyxID via a Self-Hosted Deployment, the operator of that deployment — not ChronoAI — is the controller (or equivalent) of your personal data, and your use is governed by the operator's own terms and privacy policy. Operators of Self-Hosted Deployments are themselves Users of the open-source software and are responsible for their own legal and regulatory compliance toward their end users. Operator-facing guidance is published in the project repository (see `docs/TELEMETRY_OPERATORS.md` and related documentation).

## 9. INTELLECTUAL PROPERTY AND USER CONTENT

### 9.1 Ownership

The App, its underlying software, architecture, AI models and systems, and all content, trademarks, logos, design, text, and other proprietary materials available through the Services (collectively, "**NyxID Content**") are owned by ChronoAI Pte. Ltd. or licensed to ChronoAI by third-party licensors. All intellectual property rights in the NyxID Content are reserved. Nothing in these Terms shall be construed as transferring any intellectual property rights to you. Components of the App that are made available under open-source licences are licensed to you separately under the terms of the applicable open-source licence, which prevails over this Section to the extent of any conflict.

### 9.2 Limited Licence

Subject to your compliance with these Terms, ChronoAI grants you a limited, non-exclusive, non-sublicensable, non-transferable, revocable licence to access and use the App and NyxID Content for your **personal or internal business use** in connection with the Services as permitted by these Terms. If you are accessing the Services on behalf of an organisation, the licence extends to authorised end users within that organisation, subject to your compliance with these Terms.

### 9.3 Restrictions

You agree that you will not, and will not permit any third party to:

- reproduce, modify, adapt, translate, distribute, publicly display, sell, lease, reverse engineer, decompile, or disassemble the App or any NyxID Content, except to the extent permitted by applicable open-source licence terms governing components of the App;
- use any NyxID Content for purposes outside the licence granted in Section 9.2 without ChronoAI's prior written consent;
- attempt to circumvent, disable, or interfere with any security or access control feature of the App;
- remove or obscure any proprietary rights notices in or accompanying the NyxID Content;
- use the App's AI systems or APIs to develop competing products or services.

### 9.4 User-Generated Content

To the extent you submit any content, feedback, suggestions, or information to ChronoAI through the App ("**User Content**"), you hereby grant ChronoAI a non-exclusive, royalty-free, worldwide licence — for the duration of, and as required for, the operation, maintenance, debugging, security investigation, and improvement of the Services within ChronoAI's own systems — to use, reproduce, modify, and incorporate such User Content into the Services. ChronoAI will not use your User Content to train AI models for resale or external distribution without your separate, informed consent. You represent and warrant that you have all rights necessary to grant this licence. ChronoAI's right to use your User Content terminates upon your deletion of the relevant content or your account, except to the extent the content has been incorporated into materials already published or distributed prior to deletion, and except for retention required by law or for backup purposes.

### 9.5 Copyright Complaints (DMCA and Equivalent)

ChronoAI respects the intellectual property rights of others. If you believe that material accessible on or through the App infringes your copyright, please send a written notice to **contact@chrono-ai.fun** with the following information: (i) a physical or electronic signature of the person authorised to act on behalf of the owner; (ii) identification of the copyrighted work claimed to have been infringed; (iii) identification of the material claimed to be infringing and information reasonably sufficient to permit ChronoAI to locate the material; (iv) your contact information; (v) a statement that you have a good-faith belief that the use is not authorised by the copyright owner, its agent, or the law; and (vi) a statement, made under penalty of perjury, that the information in the notice is accurate and that you are the copyright owner or are authorised to act on the owner's behalf. Counter-notices may be submitted to the same address. ChronoAI may, in appropriate circumstances and at its discretion, terminate the accounts of Users who are repeat infringers.

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

ChronoAI reserves the right to unilaterally suspend or terminate your access to all or any part of the Services. ChronoAI will provide reasonable advance notice where practicable, except that no advance notice is required where (a) you are in material breach of these Terms; (b) immediate suspension is necessary to protect other Users, ChronoAI's systems, or the integrity of the Services from a security threat; or (c) applicable law, regulation, or order requires immediate action. The grounds for suspension or termination include:

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
- provisions of these Terms that by their nature should survive termination (including Sections 3.4 (Developer Integrations and SDK Use), 4 (User-Operated AI Agents), 5 (User Rights and Obligations), 6 (Security Disclaimers), 7 (Data Protection and Privacy), 8 (Third-Party Integrations), 9 (Intellectual Property and User Content), 11 (Incident and Breach Handling), 12 (Disclaimer and Limitation of Liability), 13 (Representations and Warranties), 14 (Fees, Payment, Refunds and Auto-Renewal), 15 (Arbitration and Class Action Waiver), and 16 (Miscellaneous)) shall survive.

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

## 14. FEES, PAYMENT, REFUNDS AND AUTO-RENEWAL

### 14.1 Applicable Fees

As at the Effective Date of these Terms, ChronoAI does not charge fees for use of the App. ChronoAI reserves the right to introduce fees ("**Service Fees**") for access to all or part of the Services in the future, in which case such fees will be disclosed to you prior to your incurring them. Service Fees may include subscription fees, per-use charges, or other pricing structures as notified by ChronoAI from time to time. Sections 14.2 through 14.5 apply only if and to the extent ChronoAI has introduced Service Fees that apply to you.

### 14.2 Payment Obligations

You agree to pay all applicable Service Fees in a timely manner. ChronoAI reserves the right to suspend or terminate your access to the Services if you fail to pay any fees when due. All in-app purchases and payment processing are subject to the terms and conditions of the applicable App Platform's payment system (e.g., Apple In-App Purchase or Google Play Billing). ChronoAI is not responsible for payment processing errors, disputes, or refunds originating from App Platform payment systems.

### 14.3 Changes to Fees

ChronoAI reserves the right to modify its pricing and Service Fees at any time, with reasonable prior notice to Users.

### 14.4 Refunds

Except where required by applicable consumer law, all Service Fees are non-refundable once paid. Where a refund is requested in connection with an in-app purchase, the request must be directed to the relevant App Platform's refund process. Where applicable consumer law (including, in the European Union, Directive 2011/83/EU on consumer rights) grants a statutory right of withdrawal from a digital service contract, ChronoAI honours that right in accordance with the procedures set out in the App or notified to you on request to **contact@chrono-ai.fun**.

### 14.5 Auto-Renewal

If you purchase a subscription, the subscription will automatically renew at the end of each billing cycle at the then-current fee, unless cancelled before the renewal date. You may cancel renewal at any time via the App or, where the subscription was purchased through an App Platform, via the App Platform's subscription management interface. ChronoAI will provide reminders of upcoming renewals where required by applicable law (including the California Automatic Renewal Law and equivalent laws in other jurisdictions).

## 15. BINDING ARBITRATION AND CLASS ACTION WAIVER

> PLEASE READ THIS SECTION CAREFULLY — IT MAY SIGNIFICANTLY AFFECT YOUR LEGAL RIGHTS, INCLUDING YOUR RIGHT TO FILE A LAWSUIT IN COURT AND YOUR ABILITY TO BRING A CLASS ACTION.

### 15.1 Binding Arbitration

Any dispute, claim, or controversy ("**Claim**") relating in any way to this Agreement or your use of the App will, to the maximum extent permitted by applicable law, be resolved by binding arbitration rather than in court, except that you may assert claims in small claims court if your claims qualify.

### 15.2 Governing Law and Forum (Singapore — Sole Jurisdiction)

This Agreement and any Claim (including non-contractual disputes or claims) arising out of or in connection with it, or its subject matter or formation, shall be governed by and construed in accordance with the laws of Singapore, regardless of where you reside or where you access the App. Any Claim shall be submitted first to mediation in accordance with the Singapore International Arbitration Centre ("**SIAC**") Mediation Rules. If the dispute is not settled by mediation within fourteen (14) days of commencement, it shall be referred to and finally resolved by arbitration under the SIAC Rules. The arbitration tribunal shall consist of a single arbitrator, appointed by agreement of the parties, or failing agreement, by the President of the Court of Arbitration of SIAC. The seat of arbitration shall be Singapore. The language shall be English.

To the extent that mandatory consumer-protection or data-protection laws of your country of residence cannot be lawfully waived by contract, nothing in this Section 15.2 prevents you from invoking those mandatory protections. Subject to that reservation, you and ChronoAI agree that Singapore courts and SIAC arbitration are the exclusive forum for the resolution of any Claim arising under or in connection with these Terms.

### 15.3 Class Action Waiver

YOU AND CHRONOAI AGREE THAT EACH PARTY MAY BRING CLAIMS AGAINST THE OTHER ONLY ON AN INDIVIDUAL BASIS, AND NOT AS A PLAINTIFF OR CLASS MEMBER IN ANY PURPORTED CLASS, COLLECTIVE, OR REPRESENTATIVE PROCEEDING. THE PARTIES EXPRESSLY WAIVE ANY RIGHT TO FILE A CLASS ACTION OR SEEK RELIEF ON A CLASS BASIS. If a court of competent jurisdiction determines that this class action waiver is void or unenforceable as to a particular claim, the arbitration provisions shall not apply to that claim, and it must be brought in a court of competent jurisdiction.

## 16. MISCELLANEOUS

### 16.1 Assignment

You may not assign or transfer this Agreement or any of your rights or obligations hereunder without ChronoAI's prior written consent. ChronoAI may assign this Agreement without your consent in connection with a merger, acquisition, sale of all or substantially all of its assets, or corporate reorganisation. Any such assignment shall not relieve ChronoAI or its successor of any obligation owed to you under applicable data protection law, and Users will be notified of any change in controller in accordance with applicable transparency obligations. Any purported assignment in violation of this Section shall be void.

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

---

## Appendix A — Known open items for Legal counsel review

> **Scope-revision note (11 May 2026):** After this appendix was first written, business decisions removed Section 4 (AI Features), Section 7.6 (US state-specific privacy rights), Section 15.3 (US Texas arbitration), and Section 15.4 (UK LCIA arbitration) from the body of the document, and replaced Section 4 with a short "User-Operated AI Agents (BYOK)" section. Appendix items below that refer specifically to those removed sections — including **A.2 #4** (EU AI Act risk classification), **A.2 #13** (CCPA / CPRA sensitive-personal-information rights), and the **A.3 entries** for §4.1 AI features, §4.6 AI Act classification, §3.2 / §4.2 LLM credential isolation, §7.6 UOOM / GPC honouring, and §15.3 US arbitration opt-out — are no longer applicable and may be disregarded. Counsel determinations and engineering verifications relating to all other sections continue to apply.

The following items were surfaced during pre-commit specialist review and require attention from licensed counsel before this document is published. They are listed here so reviewers, future maintainers, and Legal have a single reference. Items below are *not* commitments by ChronoAI; they are open questions or pending verifications.

### A.1 Pending Legal input (factual / corporate)
1. **ChronoAI Pte. Ltd. Singapore UEN** — confirm and replace `[Legal: confirm UEN]` in Section 2.
2. **Joint Controller Agreement essence disclosure (GDPR Art. 26)** — confirm whether any Partner Application JCA currently exists; if so, name the partner and surface the essence in this document and the Privacy Policy. See §1.2.
3. **Registered office address** — verify "8 Marina Boulevard, #14-02, Singapore 018981" against the current ACRA register.

### A.2 Counsel determinations required
4. **EU AI Act risk classification (§4.6).** Determine whether automated risk scoring or anomaly detection falls within Annex III of Regulation (EU) 2024/1689 (e.g., (1)(d) biometric categorisation or (5)(a) access to essential services). The classification drives conformity-assessment, technical documentation, and post-market monitoring obligations.
5. **Liability cap enforceability (§12.4).** Review the USD 100 / 12-month-fees cap against EU UCTD Annex 1(b), UK Consumer Rights Act 2015 §62, Singapore Unfair Contract Terms Act §11, California Civil Code §1668, and German BGB §307 for free-tier consumer users. Consider jurisdiction-specific minimum floors.
6. **Arbitration and class-action waiver enforceability (§15).** Review enforceability under EU UCTD Annex 1(q), Rome I Art. 6, Brussels I bis Arts. 17–19, California *McGill / Brice* progeny, AB 51, PAGA, and the UK Consumer Rights Act §62. Confirm the U.S. opt-out window added in §15.3 is sufficient.
7. **Choice-of-law conflict with mandatory consumer-protection law (§15.2, §15.4).** Confirm Rome I Art. 6(2) reservation of consumer's home-jurisdiction mandatory rules.
8. **Class-action waiver poison-pill (§15.3, formerly §15.5).** Review the severability fallback that routes void-waiver claims to court; advise whether to retain or replace with a blue-pencil approach.
9. **Indemnity scope (§5.6, §13, §3.4).** Narrow developer and User indemnities to materiality and first-party-fault claims to survive EU UCTD scrutiny.
10. **SDK licence model (§9.2 vs §3.4).** Confirm whether SDK developers operate under the §9.2 limited licence, a separate developer agreement, or open-source licence terms accompanying the SDK packages.
11. **DMCA designated agent (§9.5).** Register a designated agent with the U.S. Copyright Office and confirm: 10–14 day counter-notice waiting period; safe-harbor takedown timeline; written repeat-infringer policy.
12. **California ARL clear-and-conspicuous disclosure (§14.5).** Confirm the live subscription flow meets ARL requirements (online-cancel mechanism, reminder timing, acknowledgement at point of sale).
13. **CCPA/CPRA sensitive personal information limit-of-use rights (§7.6).** Advise on inclusion of explicit SPI limit-of-use disclosure and Do-Not-Sell/Share link practice.
14. **Apple EULA template strict alignment (§8.6).** Compare current text against the latest Apple Developer Program Licence Agreement template; confirm completeness against any updates.
15. **PDPA breach-notification threshold (§11.2).** Confirm the "significant harm" plus 500-data-subjects rule applies and that the document accurately describes notification format and timing.
16. **EU consumer right-of-withdrawal Art. 16(m) carve-out (§14.4).** Confirm acknowledgement for digital content where consumer consents to immediate performance is correctly surfaced in checkout.
17. **Biometric data and BIPA exposure (§5.2, §6.4).** Confirm no biometric template ever transits NyxID systems; if any biometric processing occurs server-side, BIPA / Texas / Washington notice and consent compliance must be added.

### A.3 Pending engineering verification (factual claims in this document)
The specialist reviewer flagged the following factual claims for code-vs-doc audit prior to publication. Each should be verified against the codebase before this draft moves from "Legal review" to public-facing.

- §3.2 — proxied request and response bodies "are buffered in memory … and are not written to disk or persistently logged."
- §3.2, §4.2 — credential inputs to the AI chat assistant "are encrypted locally and are not transmitted to the underlying LLM."
- §4.1 — Automated Risk Scoring and Anomaly Detection presence and scope (verify whether shipped or aspirational; current language softened to "where enabled").
- §5.1 — data export functionality availability and scope.
- §7.5 — TLS 1.2+ minimum across all surfaces (verify server configuration).
- §7.5 — audit logging of all credential access events.
- §8.5 — Mailchimp / PostHog sub-processor list accuracy.
- §11.2 — documented incident-response procedure exists.
- §14.5 — subscription/billing implementation matches stated auto-renewal terms.

### A.4 Out-of-scope dependencies (tracked separately, do not block this draft)
These do not block this draft from being committed for Legal review, but block public publication of the Terms at `/terms`:

- Privacy Policy SCCs / EU→US transfer-basis text (Issue #499 item 1).
- Privacy Policy sub-processor list (Issue #499 item 3).
- Registration consent checkbox, age affirmation, and server-side consent record (Issue #499 item 5).
- `/terms` route registration in `frontend/src/router.tsx`.
- Mobile app ToS surface (Issue #499 item 7).
- Breach notification SOP (Issue #499 item 4).
- PostHog DSAR SOP (Issue #499 item 8).
