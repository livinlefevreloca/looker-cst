# Whether the login came from a new ISP or region.
dimension: is_new_location {
  type: yesno
  description: "True if the ISP or region differs from the profile's prior login."
  sql: ${is_new_isp} OR ${is_new_region} ;;
}
