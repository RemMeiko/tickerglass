/** 自选股列表为空时的引导。不显示空白板，也不崩溃 */
export default function EmptyHint() {
  return (
    <div className="empty-hint">
      <div className="empty-title">暂无自选股</div>
      <div className="empty-sub">双击添加自选股</div>
    </div>
  );
}
