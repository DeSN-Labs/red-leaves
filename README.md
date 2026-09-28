# red-leaves
A decentralization social network

A decentralized social network built around wallet-based identities, user-controlled content distribution, peer-to-peer infrastructure, and an on-chain economic system.

## Features

1. **Wallet-based identity**

   Users can join the network directly using their wallet address as their identity, without registration or centralized accounts.

2. **Decentralized application servers**

   The network consists of a large number of blockchain nodes that can also provide application services for users.

3. **Portable user identity**

   Users can use their wallet identity to connect to different servers and switch between servers freely.

   Each user should designate a trusted **permanent server** to handle data and services related to their identity. When connected to other servers, some social capabilities may be restricted.

4. **Social interactions**

   Users can:

   * Follow other users
   * Send direct messages
   * Participate in group chats
   * Publish original content
   * Comment on other users' content

5. **Flexible content visibility**

   Creators can define the visibility of their content, such as:

   * Public / global
   * Followers only
   * Friends only
   * Specific users
   * Private

6. **User-controlled content reception**

   Users can decide what type of content they want to receive, such as:

   * Any publicly broadcast content
   * Content from followed users
   * Content from friends
   * Other customized scopes

   If a user is added to the blacklist, their content will no longer be passively delivered to the user.

7. **Content tipping**

   Users can tip creators when reading their content.

8. **Paid content**

   Creators can require users to pay before accessing specific content.

9. **Decentralized live streaming**

   Users can broadcast live streams and choose whether the stream is free or paid.

10. **Native token economy**

    All payments and settlements within the network use the network's native token.

11. **End-to-end encryption**

    Except for globally broadcast content, all content should be encrypted.

    Content intended for specific users is encrypted using the recipient's public key.

12. **Scope-limited content encryption**

    Content that needs to be restricted to a specific group or audience can use symmetric encryption such as AES.

13. **Digital asset trading**

    The network supports the trading of digital assets.

    A digital asset can be encrypted, while the decryption key is provided to the buyer after payment. The decryption key is encrypted using the buyer's public key and delivered to the buyer.

    The protocol does not provide refunds.

    For large digital assets, the encrypted data may be stored by designated storage nodes in the network.

14. **Service-provider nodes**

    The network supports different types of service nodes, including:

    * Storage provider nodes
    * Traffic relay nodes
    * Public server nodes
    * Other specialized service nodes

    These nodes can earn rewards by providing services to the network.

15. **Traffic-based payments**

    Network traffic generally requires payment.

    User-created content is normally stored only on the user's permanent server unless it is explicitly hosted by a storage provider.

    Permanent servers may be devices such as NAS systems located behind NAT.

    When users need relay services to access content, they pay for the corresponding network traffic.

16. **Controlled monetary issuance**

    The founding team retains a limited minting capability and can receive compensation through newly issued tokens.

    The issuance mechanism will be subject to predefined rules and constraints.

17. **Paid third-party storage**

    Users who choose to store their content on third-party storage nodes generally pay for the storage service.

    Storage providers can define their own storage pricing.

18. **No platform fees**

    The network does not take a percentage of user earnings.

    Payments go directly between users and service providers or creators according to the protocol rules.

---

# Technical Design

The project is currently considering [Sovereign SDK](https://github.com/Sovereign-Labs/sovereign-sdk) as the underlying framework for building the blockchain/L2 infrastructure.

## P2P Network

The goal is to use a **single P2P connection layer** between nodes.

The same P2P infrastructure should serve both:

* Blockchain/L2 networking
* Social content and data propagation

This is intended to avoid maintaining separate P2P networks for blockchain consensus and application-level communication.

## On-chain and Off-chain Data

A key part of the design is determining which data should be stored on-chain and which data should remain off-chain.

The blockchain should primarily handle data that requires:

* Verification
* Ownership
* Settlement
* Payments
* Economic incentives
* State transitions

Large or private application data should generally remain off-chain and be transmitted or stored through the decentralized P2P network.

---

# Advantages

### 1. No platform revenue sharing

Creators receive their earnings directly without the platform taking a percentage of their revenue.

### 2. No centralized content moderation

The network does not rely on centralized content deletion or platform-level censorship.

Instead, users control what content they receive and which users they interact with.

### 3. User-controlled content distribution

Content propagation is controlled by users and the decentralized network rather than by a centralized recommendation algorithm.

Users can decide which content and which users they want to receive.

### 4. Aligned incentives between the founders and users

The founding team only retains a limited amount of token issuance authority, subject to predefined rules.

The long-term value of the network depends on the health and usage of the ecosystem, creating an incentive for the founding team to maintain a stable and useful network rather than extracting value through platform fees.
