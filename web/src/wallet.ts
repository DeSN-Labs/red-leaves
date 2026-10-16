export interface Provider {
  request(args: { method: string; params?: unknown[] }): Promise<unknown>;
  on?(event: string, callback: (...args: unknown[]) => void): void;
  removeListener?(event: string, callback: (...args: unknown[]) => void): void;
}
declare global {
  interface Window {
    ethereum?: Provider;
  }
}
export async function connectWallet() {
  const provider = window.ethereum;
  if (!provider)
    throw new Error(
      "没有检测到钱包扩展，请安装 MetaMask 等兼容钱包，或先体验演示。",
    );
  const result = await provider.request({ method: "eth_requestAccounts" });
  if (
    !Array.isArray(result) ||
    typeof result[0] !== "string" ||
    !/^0x[a-fA-F0-9]{40}$/.test(result[0])
  )
    throw new Error("钱包未返回有效地址，请重试。");
  return result[0];
}
export const shortAddress = (address: string) =>
  `${address.slice(0, 6)}…${address.slice(-4)}`;
