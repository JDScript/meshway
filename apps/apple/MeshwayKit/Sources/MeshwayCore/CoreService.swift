import MeshwayBindings

public struct CoreBuildInfo: Sendable, Equatable {
  public let version: String
  public let configurationSchema: UInt32
  public let maximumRoutingRules: UInt32
}

public protocol CoreService: Sendable {
  func buildInfo() -> CoreBuildInfo
  func isValidTagSelector(_ value: String) -> Bool
}

public struct RustCoreService: CoreService {
  public init() {}
  public func buildInfo() -> CoreBuildInfo {
    let info = coreInfo()
    return CoreBuildInfo(
      version: info.version, configurationSchema: info.configurationSchema,
      maximumRoutingRules: info.maximumRoutingRules)
  }
  public func isValidTagSelector(_ value: String) -> Bool {
    validTagSelector(value: value)
  }
}
