export default function AboutPanel() {
  return (
    <div className="panel">
      <div className="about-title">TickerGlass</div>
      <div className="hint-text">版本 0.1.0</div>

      <div className="group-title">数据来源</div>
      <p className="hint-text">
        行情与分时数据来自腾讯财经与东方财富的公开接口，均为延时数据。
        本应用不发送任何遥测，仅访问硬编码白名单内的行情域名。
      </p>

      <div className="group-title">免责声明</div>
      <p className="hint-text">
        本应用仅供个人查看行情参考，数据可能延迟、中断或与交易所口径不一致，
        <b>不构成任何投资建议</b>。请勿将本应用的数据用于实际交易决策，
        投资决策请以券商官方行情与公告为准。
      </p>

    </div>
  );
}
