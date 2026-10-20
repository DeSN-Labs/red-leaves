export interface Post {
  id: number;
  title: string;
  author: string;
  avatar: string;
  color: string;
  publishedAt: string;
  image: string;
  time: string;
  duration?: string;
  description: string;
  likes: number;
}
export const posts: Post[] = [
  {
    id: 1,
    title: "离线的一天，去山野里找回自己的节奏",
    author: "山间来信",
    avatar: "山",
    color: "#758c73",
    publishedAt: "2026-10-19T08:00:00+08:00",
    image: "photo-1441974231531-c6227db76b6e",
    time: "2 小时前",
    duration: "08:24",
    description:
      "把通知关掉，听风穿过树林。记录一次没有目的地的徒步，以及那些很容易被忽略的瞬间。",
    likes: 128,
  },
  {
    id: 2,
    title: "把生活搬进小院，慢慢过一个秋天",
    author: "阿禾的日常",
    avatar: "禾",
    color: "#bc8e69",
    publishedAt: "2026-10-19T06:00:00+08:00",
    image: "photo-1449247709967-d4461a6a6103",
    time: "4 小时前",
    duration: "12:06",
    description:
      "一杯咖啡、一张旧桌子，还有刚刚好的阳光。分享属于普通日子的片刻。",
    likes: 86,
  },
  {
    id: 3,
    title: "我的第一台家庭 NAS：数据终于回到了自己手里",
    author: "字节花园",
    avatar: "字",
    color: "#71899b",
    publishedAt: "2026-10-19T05:00:00+08:00",
    image: "photo-1498050108023-c5249f4df085",
    time: "5 小时前",
    duration: "16:32",
    description:
      "从备份照片开始，搭建一个属于自己的数字空间。聊聊设备、存储，以及为什么想拥有自己的数据。",
    likes: 234,
  },
  {
    id: 4,
    title: "海边没有信号，却有很多想说的话",
    author: "不赶路的人",
    avatar: "路",
    color: "#71999d",
    publishedAt: "2026-10-18T18:00:00+08:00",
    image: "photo-1475924156734-496f6cac6ec1",
    time: "昨天",
    duration: "06:18",
    description: "沿着海岸走到日落。用镜头收藏海浪，也把这份安静分享给你。",
    likes: 192,
  },
  {
    id: 5,
    title: "周末的餐桌，是我喜欢的生活形状",
    author: "阿禾的日常",
    avatar: "禾",
    color: "#bc8e69",
    publishedAt: "2026-10-18T12:00:00+08:00",
    image: "photo-1490645935967-10de6ba17061",
    time: "昨天",
    description:
      "去市场买些新鲜蔬菜，认真做一顿饭。不必复杂，和喜欢的人一起就好。",
    likes: 57,
  },
  {
    id: 6,
    title: "在城市的缝隙里，收集一点绿色",
    author: "山间来信",
    avatar: "山",
    color: "#758c73",
    publishedAt: "2026-10-18T09:00:00+08:00",
    image: "photo-1416879595882-3373a0480b5b",
    time: "昨天",
    duration: "05:42",
    description:
      "阳台上的小小花园又长出了新叶。即使生活在城市，也可以给自然留一个位置。",
    likes: 104,
  },
];
export const photo = (id: string, width = 900) =>
  `https://images.unsplash.com/${id}?auto=format&fit=crop&w=${width}&q=80`;
