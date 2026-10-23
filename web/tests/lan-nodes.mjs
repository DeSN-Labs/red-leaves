// Optional remote lifecycle for delivery.anvil.mjs. SSH authentication is managed outside this file.
import { readFile } from "node:fs/promises";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { isIP } from "node:net";
const execute = promisify(execFile);
const quote = value => `'${String(value).replaceAll("'", "'\\''")}'`;

export async function lanNodes(profile, child, localPort, rpc) {
  if (!profile) return null;
  const values = Object.fromEntries((await readFile(profile, "utf8")).split(/\r?\n/)
    .map(line => line.trim()).filter(line => line && !line.startsWith("#"))
    .map(line => { const split = line.indexOf("="); if (split < 1) throw new Error("Invalid LAN profile"); return [line.slice(0, split), line.slice(split + 1)]; }));
  const hosts = values.LAN_HOSTS?.split(",");
  if (hosts?.length !== 3 || hosts.some(host => isIP(host) !== 4)) throw new Error("LAN_HOSTS requires three IPv4 SSH hosts");
  const user = values.LAN_SSH_USER ?? "loop";
  if (!/^[a-zA-Z0-9_-]+$/.test(user)) throw new Error("Invalid LAN SSH user");
  const root = values.LAN_REMOTE_ROOT ?? "/home/loop/red-leaves-lan-test";
  const sockets = values.LAN_SSH_CONTROL_DIR ?? "/tmp";
  const direct = values.LAN_TRANSPORT === "direct";
  if (values.LAN_TRANSPORT && !["relay", "direct"].includes(values.LAN_TRANSPORT)) throw new Error("Invalid LAN transport");
  const run = `run-${Date.now()}`;
  const rpcPort = Number(values.LAN_RPC_PORT ?? 18545);
  const args = host => ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-S", `${sockets}/red-leaves-lan-${host}.sock`, `${user}@${host}`];
  const remote = (host, command, input) => new Promise((resolve, reject) => {
    const process = execFile("ssh", [...args(host), command], { timeout: 30000 }, (error, stdout, stderr) => error ? reject(new Error(`Remote command failed on ${host}: ${stderr || error.message}`)) : resolve(stdout));
    process.stdin.end(input);
  });
  if (!Number.isInteger(rpcPort) || rpcPort < 1024 || rpcPort > 65535) throw new Error("Invalid LAN RPC port");
  const apis = [];
  const forwardsReady = await Promise.allSettled(hosts.map(async host => {
    const port = await localPort();
    const forwards = ["-L", `127.0.0.1:${port}:127.0.0.1:3000`, "-R", `127.0.0.1:${rpcPort}:127.0.0.1:${new URL(rpc).port}`];
    if (host === hosts[0]) {
      const relayPort = await localPort();
      forwards.push("-L", `127.0.0.1:${relayPort}:127.0.0.1:3001`);
      values.relayApi = `http://127.0.0.1:${relayPort}`;
    }
    await execute("ssh", ["-O", "forward", "-o", "ExitOnForwardFailure=yes", ...forwards, ...args(host)]);
    const target = { host, api: `http://127.0.0.1:${port}`, forwards };
    apis.push(target);
    return target;
  }));
  async function cleanup() {
    for (const target of apis) await execute("ssh", ["-O", "cancel", ...target.forwards, ...args(target.host)]).catch(error => console.error(`Unable to cancel SSH forwards: ${error.message}`));
  }
  const failure = forwardsReady.find(result => result.status === "rejected");
  if (failure) { await cleanup(); throw failure.reason; }
  apis.sort((a, b) => hosts.indexOf(a.host) - hosts.indexOf(b.host));
  return {
    direct,
    relayAddress(peer) { return `/ip4/${hosts[0]}/tcp/4002/p2p/${peer}`; },
    async start(name, relay, registry) {
      const target = name === "relay" ? apis[0] : apis[Number(name.split("-").at(-1))];
      const path = `${root}/${run}/${name}`;
      const api = name === "relay" ? values.relayApi : target.api;
      const p2pPort = name === "relay" ? 4002 : 4001;
      const config = {
        RED_LEAVES_LISTEN: `127.0.0.1:${name === "relay" ? 3001 : 3000}`,
        RED_LEAVES_DATABASE_URL: `sqlite://${path}/nas.sqlite`,
        RED_LEAVES_AUTH_URI: "http://127.0.0.1:5173",
        RED_LEAVES_NODE_KEY_PATH: `${path}/node.key`,
        RED_LEAVES_RPC_URL: `http://127.0.0.1:${rpcPort}`,
        RED_LEAVES_USER_REGISTRY_ADDRESS: registry,
        RED_LEAVES_CONTENT_PATH: `${path}/content`,
        RED_LEAVES_MASTER_KEY_PATH: `${path}/master.key`,
        RED_LEAVES_USER_KEY_PATH: `${path}/keys`,
        RED_LEAVES_P2P_ENABLED: "true",
        RED_LEAVES_P2P_LISTEN: `/ip4/0.0.0.0/tcp/${p2pPort}`,
        RED_LEAVES_P2P_ADVERTISE: `/ip4/${target.host}/tcp/${p2pPort}`,
        RED_LEAVES_P2P_RELAYS: relay ?? "",
        RED_LEAVES_P2P_PEERS: "",
        RED_LEAVES_P2P_RELAY_SERVER: name === "relay" ? "true" : "false",
        RED_LEAVES_LOG_DIR: `${path}/logs`,
      };
      await remote(target.host, `umask 077; mkdir -p ${quote(path)}; cat > ${quote(`${path}/.env.profile`)}`, Object.entries(config).map(([key, value]) => `${key}=${value}`).join("\n") + "\n");
      const node = { api, path, host: target.host };
      node.restart = async () => {
        node.process = child("ssh", [...args(target.host), `cd ${quote(path)} && echo $$ > node.pid && exec ${quote(`${root}/bin/red-leaves`)}`]);
        node.process.stop = async () => { await remote(target.host, `cd ${quote(path)} && kill -TERM "$(cat node.pid)"`); };
        for (let attempt = 0; attempt < 150; attempt++) {
          if (node.process.exitCode !== null) throw new Error(`NAS startup failed on ${target.host}: ${node.process.log}`);
          try { if ((await fetch(`${api}/api/auth/me`)).status === 401) return; } catch {}
          await new Promise(resolve => setTimeout(resolve, 200));
        }
        throw new Error(`NAS startup timed out on ${target.host}`);
      };
      await node.restart();
      return node;
    },
    async connect(nodes, stop) {
      for (const node of nodes) {
        const peers = nodes.filter(other => other !== node).map(other => `/ip4/${other.host}/tcp/4001/p2p/${other.peer}`).join(",");
        await remote(node.host, `sed -i '/^RED_LEAVES_P2P_PEERS=/d' ${quote(`${node.path}/.env.profile`)}; printf '%s\n' ${quote(`RED_LEAVES_P2P_PEERS=${peers}`)} >> ${quote(`${node.path}/.env.profile`)}`);
      }
      for (const node of nodes) { await stop(node.process); await node.restart(); }
    },
    cleanup,
    async report() {
      const result = { run, hosts, root, transport: direct ? "LAN libp2p direct connections" : "LAN libp2p relay circuits" };
      console.log(JSON.stringify(result));
      return result;
    },
  };
}
