// Transitional vanilla-JS driver for the setup/login forms. Replaced by the
// Svelte auth UI when it lands. Posts JSON to the auth API and reports status inline.
async function post(url, payload) {
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(payload),
  });
  return res;
}

function fields(form) {
  return Object.fromEntries(new FormData(form).entries());
}

const setupForm = document.getElementById("setup-form");
if (setupForm) {
  setupForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    const f = fields(setupForm);
    const body = { username: f.username, password: f.password };
    if (f.token) body.token = f.token;
    const res = await post("/api/setup", body);
    document.getElementById("msg").textContent = res.ok
      ? "Admin created. You can now log in."
      : `Setup failed (${res.status}).`;
    if (res.ok) window.location.href = "/login.html";
  });
}

const loginForm = document.getElementById("login-form");
if (loginForm) {
  loginForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    const res = await post("/api/login", fields(loginForm));
    document.getElementById("msg").textContent = res.ok
      ? "Logged in."
      : "Invalid username or password.";
    if (res.ok) window.location.href = "/";
  });
}
