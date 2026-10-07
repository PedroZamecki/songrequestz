// Conventional commits, short subject only: "type(scope)?: subject".
export default {
  extends: ["@commitlint/config-conventional"],
  rules: {
    "header-max-length": [2, "always", 72],
    "body-empty": [2, "always"],
    "footer-empty": [2, "always"],
    "trailer-exists": [0],
  },
};
